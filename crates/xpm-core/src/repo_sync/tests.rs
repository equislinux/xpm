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
        database_sig_level: None,
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
        database_sig_level: None,
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
        database_sig_level: Some(SigLevel::Required),
    };

    let keyring = temp.path().join("trustedkeys.gpg");
    let err = sync_repo_databases(&repo, "x86_64", &sync, 2, SigLevel::Optional, &keyring)
        .expect_err("sync must fail without .sig in required mode");

    let msg = format!("{err}");
    assert!(msg.contains("signature required"));
}

#[test]
fn local_file_repo_sync_required_sig_fails_on_invalid_signature() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mirror = temp.path().join("mirror");
    let sync = temp.path().join("sync");
    fs::create_dir_all(&mirror).expect("create mirror dir");

    fs::write(mirror.join("core.db"), b"db-data").expect("write db");
    fs::write(mirror.join("core.db.sig"), b"not-a-valid-signature").expect("write bad sig");

    let keyring = temp.path().join("trustedkeys.gpg");
    fs::write(&keyring, b"").expect("create keyring");

    let repo = Repository {
        name: "core".to_string(),
        server: vec![format!("file://{}", mirror.display())],
        sig_level: Some(SigLevel::Required),
        database_sig_level: Some(SigLevel::Required),
    };

    let err = sync_repo_databases(&repo, "x86_64", &sync, 2, SigLevel::Optional, &keyring)
        .expect_err("sync must fail with invalid signature in required mode");

    let msg = format!("{err}");
    assert!(msg.contains("signature verification failed"));
}

#[test]
fn local_file_package_verify_required_sig_fails_on_invalid_signature() {
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

    let keyring = temp.path().join("trustedkeys.gpg");
    fs::write(&keyring, b"").expect("create keyring");

    let sig_url = format!("file://{}", sig_path.display());
    let err = verify_remote_signature(&package_dest, &sig_url, SigLevel::Required, &keyring, 2)
        .expect_err("install package verification must fail with invalid signature");

    let msg = format!("{err}");
    assert!(msg.contains("signature verification failed"));
}

#[test]
fn package_failover_uses_next_mirror_after_a_missing_file() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mirror = temp.path().join("mirror");
    fs::create_dir_all(&mirror).expect("create mirror dir");
    let package = mirror.join("demo-1.0-1-x86_64.pkg.tar.zst");
    fs::write(&package, b"payload").expect("write package");

    let missing = format!("file://{}", temp.path().join("missing.pkg").display());
    let good = format!("file://{}", package.display());
    let dest = temp.path().join("downloaded.pkg");

    let selected = download_first_available(&[missing, good.clone()], &dest, 3)
        .expect("second mirror should be used");
    assert_eq!(selected, good);
    assert_eq!(fs::read(dest).unwrap(), b"payload");
}

#[test]
fn checksum_failure_falls_over_to_the_next_mirror() {
    let temp = tempfile::tempdir().expect("tempdir");
    let bad = temp.path().join("bad.pkg");
    let good = temp.path().join("good.pkg");
    fs::write(&bad, b"<html>mirror error</html>").expect("write bad package");
    fs::write(&good, b"package-payload").expect("write good package");
    let expected = format!("{:x}", Sha256::digest(b"package-payload"));
    let dest = temp.path().join("download.pkg");

    let selected = download_first_available_checked(
        &[
            format!("file://{}", bad.display()),
            format!("file://{}", good.display()),
        ],
        &dest,
        1,
        Some(&expected),
    )
    .expect("corrupt mirror must be skipped");
    assert_eq!(selected, format!("file://{}", good.display()));
    assert_eq!(fs::read(dest).unwrap(), b"package-payload");
}

#[test]
fn package_candidates_do_not_use_homepage_for_arch_packages() {
    let repo = Repository {
        name: "extra".to_string(),
        server: vec!["https://mirror.example/$repo/os/$arch".to_string()],
        sig_level: None,
        database_sig_level: None,
    };
    let entry = RepoEntry {
        name: "brotli".to_string(),
        version: "1.2.1-1".to_string(),
        filename: Some("brotli-1.2.1-1-x86_64.pkg.tar.zst".to_string()),
        sha256sum: None,
        url: Some("https://github.com/google/brotli".to_string()),
        ..Default::default()
    };

    let candidates = package_download_candidates(&repo, "x86_64", &entry);
    assert_eq!(
        candidates,
        vec!["https://mirror.example/extra/os/x86_64/brotli-1.2.1-1-x86_64.pkg.tar.zst"]
    );
}

#[test]
fn package_candidates_include_server_and_github_release() {
    let repo = Repository {
        name: "x".to_string(),
        server: vec!["https://equislinux.github.io/x-repo/x/$arch".to_string()],
        sig_level: None,
        database_sig_level: None,
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
