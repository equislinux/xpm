//! Remote repository database synchronization.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::config::{Repository, SigLevel};
use crate::repo_db::RepoEntry;
use crate::signing::{load_keyring, verify_file, VerifyOutcome};
use crate::{XpmError, XpmResult};
use sha2::{Digest, Sha256};

/// Result metadata for a repository sync operation.
#[derive(Debug, Clone)]
pub struct RepoSyncResult {
    pub mirror: String,
    pub db_downloaded: bool,
    pub files_downloaded: bool,
}

/// Expand `$repo` and `$arch` placeholders in a repository server URL.
pub fn expand_repo_url(server: &str, repo: &str, arch: &str) -> String {
    server.replace("$repo", repo).replace("$arch", arch)
}

/// Download `<repo>.db` and optional `<repo>.files` into `sync_dir`.
///
/// The operation tries each configured mirror until one succeeds for `.db`.
/// The `.files` download is best-effort and does not fail the whole sync.
pub fn sync_repo_databases(
    repo: &Repository,
    arch: &str,
    sync_dir: &Path,
    retries: u32,
    default_sig_level: SigLevel,
    keyring_path: &Path,
) -> XpmResult<RepoSyncResult> {
    fs::create_dir_all(sync_dir)?;

    let db_dest = sync_dir.join(format!("{}.db", repo.name));
    let files_dest = sync_dir.join(format!("{}.files", repo.name));
    let sig_level = repo.sig_level.unwrap_or(default_sig_level);

    let mut last_error: Option<XpmError> = None;

    for server in &repo.server {
        let base = expand_repo_url(server, &repo.name, arch);
        let base = base.trim_end_matches('/');
        let db_url = format!("{base}/{}.db", repo.name);
        let db_sig_url = format!("{db_url}.sig");

        match download_with_retries(&db_url, &db_dest, retries) {
            Ok(()) => {
                verify_remote_signature(&db_dest, &db_sig_url, sig_level, keyring_path, retries)?;

                let files_url = format!("{base}/{}.files", repo.name);
                let files_downloaded = download_with_retries(&files_url, &files_dest, retries)
                    .map(|_| true)
                    .unwrap_or(false);

                return Ok(RepoSyncResult {
                    mirror: base.to_string(),
                    db_downloaded: true,
                    files_downloaded,
                });
            }
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        XpmError::Database(format!(
            "no usable mirror found for repository '{}'",
            repo.name
        ))
    }))
}

pub fn verify_remote_signature(
    artifact_path: &Path,
    sig_url: &str,
    sig_level: SigLevel,
    keyring_path: &Path,
    retries: u32,
) -> XpmResult<()> {
    if sig_level == SigLevel::Never {
        return Ok(());
    }

    let sig_dest = signature_cache_path(artifact_path);

    if let Err(err) = download_with_retries(sig_url, &sig_dest, retries) {
        if sig_level == SigLevel::Required {
            return Err(XpmError::SignatureError(format!(
                "signature required but could not be downloaded from {sig_url}: {err}"
            )));
        }

        tracing::warn!(
            artifact = %artifact_path.display(),
            sig_url,
            error = %err,
            "signature unavailable in optional mode; continuing"
        );
        return Ok(());
    }

    let certs = match load_keyring(keyring_path) {
        Ok(certs) => certs,
        Err(err) => {
            if sig_level == SigLevel::Required {
                return Err(XpmError::SignatureError(format!(
                    "failed to load keyring {}: {err}",
                    keyring_path.display()
                )));
            }

            tracing::warn!(
                artifact = %artifact_path.display(),
                keyring = %keyring_path.display(),
                error = %err,
                "could not load keyring in optional mode; continuing"
            );
            return Ok(());
        }
    };

    if certs.is_empty() {
        if sig_level == SigLevel::Optional {
            tracing::warn!(
                artifact = %artifact_path.display(),
                keyring = %keyring_path.display(),
                "keyring has no certificates in optional mode; continuing"
            );
            return Ok(());
        }

        return Err(XpmError::SignatureError(format!(
            "no certificates found in keyring {}",
            keyring_path.display()
        )));
    }

    let outcome = match verify_file(artifact_path, &sig_dest, &certs) {
        Ok(outcome) => outcome,
        Err(err) => {
            if sig_level == SigLevel::Required {
                return Err(err);
            }

            tracing::warn!(
                artifact = %artifact_path.display(),
                error = %err,
                "signature check failed in optional mode; continuing"
            );
            return Ok(());
        }
    };

    match outcome {
        VerifyOutcome::Good { key_id } => {
            tracing::debug!(
                artifact = %artifact_path.display(),
                key_id,
                "signature verified"
            );
            Ok(())
        }
        VerifyOutcome::UnknownKey => Err(XpmError::SignatureError(format!(
            "signature for {} is valid but key is unknown",
            artifact_path.display()
        ))),
        VerifyOutcome::Bad { reason } => Err(XpmError::SignatureError(format!(
            "invalid signature for {}: {}",
            artifact_path.display(),
            reason
        ))),
    }
}

fn signature_cache_path(artifact_path: &Path) -> PathBuf {
    let file_name = artifact_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    artifact_path.with_file_name(format!("{file_name}.sig"))
}

/// Build candidate package download URLs for a repository entry.
///
/// Resolution order:
/// 1) `<expanded_server>/<filename>` for each configured repo server
/// 2) If `%URL%` is a direct `.xp` URL, use it as-is
/// 3) If `%URL%` points to a GitHub repo, derive release URL:
///    `<url>/releases/download/<name>-<version>/<filename>`
pub fn package_download_candidates(
    repo: &Repository,
    arch: &str,
    entry: &RepoEntry,
) -> Vec<String> {
    let mut candidates = Vec::new();

    let Some(filename) = entry.filename.as_deref() else {
        return candidates;
    };

    for server in &repo.server {
        let base = expand_repo_url(server, &repo.name, arch);
        let base = base.trim_end_matches('/');
        candidates.push(format!("{base}/{filename}"));
    }

    if let Some(url) = entry.url.as_deref() {
        let clean = url.trim();
        if clean.ends_with(".xp") {
            candidates.push(clean.to_string());
        } else if clean.starts_with("https://github.com/") {
            let repo_url = clean.trim_end_matches('/');
            let tag = format!("{}-{}", entry.name, entry.version);
            candidates.push(format!("{repo_url}/releases/download/{tag}/{filename}"));
        }
    }

    candidates
}

/// Download from the first working URL to `dest`.
pub fn download_first_available(urls: &[String], dest: &Path, retries: u32) -> XpmResult<String> {
    let mut last_error: Option<XpmError> = None;

    for url in urls {
        match download_with_retries(url, dest, retries) {
            Ok(()) => return Ok(url.clone()),
            Err(err) => last_error = Some(err),
        }
    }

    Err(last_error.unwrap_or_else(|| XpmError::Database("no usable package URL found".to_string())))
}

/// Verify SHA-256 for a file if checksum metadata is available.
pub fn verify_sha256(path: &Path, expected_hex: &str) -> XpmResult<()> {
    let expected = expected_hex.trim();
    if expected.is_empty() {
        return Ok(());
    }

    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual = format!("{:x}", hasher.finalize());

    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(XpmError::Package(format!(
            "checksum mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        )))
    }
}

fn download_with_retries(url: &str, dest: &Path, retries: u32) -> XpmResult<()> {
    let attempts = retries.max(1);
    let mut last_error: Option<XpmError> = None;

    for attempt in 1..=attempts {
        match download_once(url, dest) {
            Ok(()) => return Ok(()),
            Err(err) => {
                tracing::warn!(url, attempt, attempts, error = %err, "download attempt failed");
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| XpmError::Database(format!("download failed: {url}"))))
}

fn download_once(url: &str, dest: &Path) -> XpmResult<()> {
    if let Some(path) = local_file_path(url) {
        tracing::debug!(url, source = %path.display(), "syncing database from local file mirror");
        return copy_local_file(&path, dest);
    }

    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| XpmError::Database(format!("http client init failed: {e}")))?;

    let mut response = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| XpmError::Database(format!("{url}: {e}")))?;

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let total = response.content_length();
    let temp_path = dest.with_extension("part");
    let mut file = fs::File::create(&temp_path)?;
    let mut downloaded: u64 = 0;
    let mut buffer = [0u8; 65536];
    let mut next_report: u64 = 25;

    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|e| XpmError::Database(format!("{url}: {e}")))?;
        if n == 0 {
            break;
        }

        file.write_all(&buffer[..n])?;
        downloaded += n as u64;

        if let Some(total) = total {
            let percent = total
                .checked_div(1)
                .map(|_| downloaded.saturating_mul(100) / total)
                .unwrap_or(0);
            if percent >= next_report {
                tracing::debug!(url, downloaded, total, percent, "download progress");
                next_report = (next_report + 25).min(100);
            }
        }
    }

    file.flush()?;
    fs::rename(&temp_path, dest)?;

    tracing::debug!(url, downloaded, dest = %dest.display(), "download complete");
    Ok(())
}

fn local_file_path(url: &str) -> Option<PathBuf> {
    let raw = url.strip_prefix("file://")?;
    if raw.is_empty() {
        return None;
    }
    Some(PathBuf::from(raw))
}

fn copy_local_file(source: &Path, dest: &Path) -> XpmResult<()> {
    let data = fs::read(source)?;
    write_atomically(dest, &data)
}

fn write_atomically(dest: &Path, bytes: &[u8]) -> XpmResult<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let temp_path = dest.with_extension("part");
    let mut file = fs::File::create(&temp_path)?;
    file.write_all(bytes)?;
    file.flush()?;
    fs::rename(&temp_path, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_repo_url_substitutes_placeholders() {
        let url = expand_repo_url(
            "https://mirror.example.com/$repo/os/$arch",
            "core",
            "x86_64",
        );
        assert_eq!(url, "https://mirror.example.com/core/os/x86_64");
    }

    #[test]
    fn local_file_repo_sync_downloads_db_and_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mirror = temp.path().join("mirror");
        let sync = temp.path().join("sync");
        fs::create_dir_all(&mirror).expect("create mirror dir");

        fs::write(mirror.join("core.db"), b"db-data").expect("write db");
        fs::write(mirror.join("core.files"), b"files-data").expect("write files");

        let repo = Repository {
            name: "core".to_string(),
            server: vec![format!("file://{}", mirror.display())],
            sig_level: None,
        };

        let keyring = temp.path().join("trustedkeys.gpg");
        std::fs::write(&keyring, b"not-a-real-keyring").expect("write keyring placeholder");

        let result = sync_repo_databases(&repo, "x86_64", &sync, 2, SigLevel::Never, &keyring)
            .expect("sync repo");
        assert!(result.db_downloaded);
        assert!(result.files_downloaded);
        assert!(sync.join("core.db").exists());
        assert!(sync.join("core.files").exists());
    }

    #[test]
    fn local_file_repo_sync_optional_sig_allows_missing_signature() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mirror = temp.path().join("mirror");
        let sync = temp.path().join("sync");
        fs::create_dir_all(&mirror).expect("create mirror dir");

        fs::write(mirror.join("core.db"), b"db-data").expect("write db");

        let repo = Repository {
            name: "core".to_string(),
            server: vec![format!("file://{}", mirror.display())],
            sig_level: Some(SigLevel::Optional),
        };

        let keyring = temp.path().join("trustedkeys.gpg");
        let result = sync_repo_databases(&repo, "x86_64", &sync, 2, SigLevel::Optional, &keyring)
            .expect("sync repo should succeed without .sig in optional mode");

        assert!(result.db_downloaded);
        assert!(sync.join("core.db").exists());
    }

    #[test]
    fn local_file_repo_sync_required_sig_fails_when_missing_signature() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mirror = temp.path().join("mirror");
        let sync = temp.path().join("sync");
        fs::create_dir_all(&mirror).expect("create mirror dir");

        fs::write(mirror.join("core.db"), b"db-data").expect("write db");

        let repo = Repository {
            name: "core".to_string(),
            server: vec![format!("file://{}", mirror.display())],
            sig_level: Some(SigLevel::Required),
        };

        let keyring = temp.path().join("trustedkeys.gpg");
        let err = sync_repo_databases(&repo, "x86_64", &sync, 2, SigLevel::Optional, &keyring)
            .expect_err("sync must fail without .sig in required mode");

        let msg = format!("{err}");
        assert!(msg.contains("signature required"));
    }

    #[test]
    fn local_file_repo_sync_required_sig_fails_on_invalid_signature() {
        use sequoia_openpgp::cert::CertBuilder;
        use sequoia_openpgp::serialize::Serialize;

        let temp = tempfile::tempdir().expect("tempdir");
        let mirror = temp.path().join("mirror");
        let sync = temp.path().join("sync");
        fs::create_dir_all(&mirror).expect("create mirror dir");

        fs::write(mirror.join("core.db"), b"db-data").expect("write db");
        fs::write(mirror.join("core.db.sig"), b"not-a-valid-signature").expect("write bad sig");

        let (cert, _) = CertBuilder::general_purpose(None, Some("Test <test@example.com>"))
            .generate()
            .expect("generate cert");

        let keyring = temp.path().join("trustedkeys.gpg");
        let mut keyring_file = fs::File::create(&keyring).expect("create keyring");
        cert.serialize(&mut keyring_file)
            .expect("write keyring cert");

        let repo = Repository {
            name: "core".to_string(),
            server: vec![format!("file://{}", mirror.display())],
            sig_level: Some(SigLevel::Required),
        };

        let err = sync_repo_databases(&repo, "x86_64", &sync, 2, SigLevel::Optional, &keyring)
            .expect_err("sync must fail with invalid signature in required mode");

        let msg = format!("{err}");
        assert!(msg.contains("signature verification failed"));
    }

    #[test]
    fn local_file_package_verify_required_sig_fails_on_invalid_signature() {
        use sequoia_openpgp::cert::CertBuilder;
        use sequoia_openpgp::serialize::Serialize;

        let temp = tempfile::tempdir().expect("tempdir");
        let mirror = temp.path().join("mirror");
        let download_dir = temp.path().join("download");
        fs::create_dir_all(&mirror).expect("create mirror dir");
        fs::create_dir_all(&download_dir).expect("create download dir");

        let package_name = "xfetch-0.1.0-1-x86_64.xp";
        let package_dest = download_dir.join(package_name);
        fs::write(&package_dest, b"package-data").expect("write package");

        let sig_path = mirror.join(format!("{package_name}.sig"));
        fs::write(&sig_path, b"not-a-valid-signature").expect("write bad sig");

        let (cert, _) = CertBuilder::general_purpose(None, Some("Test <test@example.com>"))
            .generate()
            .expect("generate cert");

        let keyring = temp.path().join("trustedkeys.gpg");
        let mut keyring_file = fs::File::create(&keyring).expect("create keyring");
        cert.serialize(&mut keyring_file)
            .expect("write keyring cert");

        let sig_url = format!("file://{}", sig_path.display());
        let err = verify_remote_signature(&package_dest, &sig_url, SigLevel::Required, &keyring, 2)
            .expect_err("install package verification must fail with invalid signature");

        let msg = format!("{err}");
        assert!(msg.contains("signature verification failed"));
    }

    #[test]
    fn package_candidates_include_server_and_github_release() {
        let repo = Repository {
            name: "x".to_string(),
            server: vec!["https://equislinux.github.io/x-repo/x/$arch".to_string()],
            sig_level: None,
        };
        let entry = RepoEntry {
            name: "xfetch".to_string(),
            version: "0.1.0-1".to_string(),
            filename: Some("xfetch-0.1.0-1-x86_64.xp".to_string()),
            sha256sum: None,
            url: Some("https://github.com/xfetch-cli/xfetch".to_string()),
            ..Default::default()
        };

        let candidates = package_download_candidates(&repo, "x86_64", &entry);
        assert!(candidates
            .iter()
            .any(|u| u == "https://equislinux.github.io/x-repo/x/x86_64/xfetch-0.1.0-1-x86_64.xp"));
        assert!(candidates
            .iter()
            .any(|u| u == "https://github.com/xfetch-cli/xfetch/releases/download/xfetch-0.1.0-1/xfetch-0.1.0-1-x86_64.xp"));
    }
}
