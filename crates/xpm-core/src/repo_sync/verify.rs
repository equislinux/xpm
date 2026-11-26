//! Signature and checksum verification for remote artifacts.

use super::*;

/// Try the detached signature from each candidate mirror until one verifies.
/// A mirror can return a package while serving an empty/broken `.sig`.
pub fn verify_remote_signature_any(
    artifact_path: &Path,
    sig_urls: &[String],
    sig_level: SigLevel,
    keyring_path: &Path,
    retries: u32,
) -> XpmResult<()> {
    if sig_level == SigLevel::Never || sig_urls.is_empty() {
        return verify_remote_signature(
            artifact_path,
            sig_urls.first().map(String::as_str).unwrap_or(""),
            sig_level,
            keyring_path,
            retries,
        );
    }
    let mut last_error = None;
    let mut unique = Vec::new();
    for url in sig_urls {
        if !unique.contains(url) {
            unique.push(url.clone());
        }
    }
    for sig_url in &unique {
        match verify_remote_signature(
            artifact_path,
            sig_url,
            sig_level,
            keyring_path,
            retries.min(1),
        ) {
            Ok(()) => return Ok(()),
            Err(err) => {
                tracing::warn!(
                    artifact = %artifact_path.display(),
                    sig_url,
                    error = %err,
                    "signature candidate failed; trying next mirror"
                );
                last_error = Some(err);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| XpmError::SignatureError("no signature URL".into())))
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
    // Optional signatures (the official Arch databases) should be attempted
    // once. Retrying every mirror for a file that is expected to be absent
    // only floods the log and slows sync.
    let attempts = if sig_level == SigLevel::Required {
        retries
    } else {
        1
    };

    let sig_warn = sig_level == SigLevel::Required;
    if let Err(err) = download_with_retries(
        sig_url,
        &sig_dest,
        attempts,
        true,
        None,
        None,
        Some(METADATA_TIMEOUT),
        sig_warn,
    ) {
        if sig_level == SigLevel::Required {
            return Err(XpmError::SignatureError(format!(
                "signature required but could not be downloaded from {sig_url}: {err}"
            )));
        }

        tracing::debug!(
            artifact = %artifact_path.display(),
            sig_url,
            error = %err,
            "signature unavailable in optional mode; continuing"
        );
        return Ok(());
    }

    let outcome = match verify_file(artifact_path, &sig_dest, keyring_path) {
        Ok(outcome) => outcome,
        Err(err) => {
            if sig_level == SigLevel::Required {
                return Err(err);
            }
            tracing::debug!(
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
