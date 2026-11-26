use super::*;
use std::io::Write;
use tempfile::TempDir;

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn write_file(root: &Path, relative: &str, data: &[u8], mode: u32) -> fs::Metadata {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent");
    }
    let mut file = File::create(&path).expect("create");
    file.write_all(data).expect("write");
    drop(file);
    let mut permissions = fs::metadata(&path).expect("metadata").permissions();
    permissions.set_mode(mode);
    fs::set_permissions(&path, permissions).expect("chmod");
    fs::metadata(&path).expect("metadata")
}

fn write_mtree(local_db: &Path, package: &str, raw: &str) {
    local_db::write_mtree(local_db, package, raw.as_bytes()).expect("mtree");
}

#[test]
fn fast_check_reports_missing_and_type_mismatches() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let local_db = tmp.path().join("local");
    let pkg_dir = local_db.join("hello");
    fs::create_dir_all(&pkg_dir).expect("db");
    fs::write(pkg_dir.join("version"), "1.0-1").expect("version");

    fs::create_dir_all(root.join("usr/bin")).expect("dirs");
    write_file(&root, "usr/bin/hello", b"hello", 0o755);
    fs::create_dir_all(root.join("usr/share")).expect("share");

    local_db::write_file_list(
        &local_db,
        "hello",
        &[
            "usr/".to_string(),
            "usr/bin/".to_string(),
            "usr/bin/hello".to_string(),
            "usr/share/".to_string(),
        ],
    )
    .expect("files");

    let report = check_package(&root, &local_db, "hello", false).expect("check");
    assert_eq!(report.total, 4);
    assert!(report.missing.is_empty());
    assert!(report.is_ok(false));

    fs::remove_file(root.join("usr/bin/hello")).expect("remove");
    fs::remove_dir(root.join("usr/share")).expect("remove dir");
    write_file(&root, "usr/share", b"now a file", 0o644);

    let report = check_package(&root, &local_db, "hello", false).expect("check");
    assert_eq!(report.missing.len(), 2);
    assert!(report.missing.contains(&"usr/bin/hello".to_string()));
    assert!(report.missing.contains(&"usr/share/".to_string()));
    assert!(!report.is_ok(false));
}

#[test]
fn full_check_verifies_hashes_modes_and_symlinks() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let local_db = tmp.path().join("local");
    let pkg_dir = local_db.join("hello");
    fs::create_dir_all(&pkg_dir).expect("db");
    fs::write(pkg_dir.join("version"), "1.0-1").expect("version");

    let binary = b"#!/bin/sh\necho hello\n";
    write_file(&root, "usr/bin/hello", binary, 0o755);
    fs::create_dir_all(root.join("usr/bin")).expect("bin dir");
    for dir in ["usr", "usr/bin"] {
        fs::set_permissions(
            root.join(dir),
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .expect("fixture directory permissions");
    }
    let target = root.join("usr/bin/hello-real");
    std::os::unix::fs::symlink("hello", &target).expect("symlink");

    let binary_hash = sha256_hex(binary);
    let mtree = format!(
        "#mtree\n\
             ./usr type=dir mode=0755 uid=0 gid=0\n\
             ./usr/bin type=dir mode=0755 uid=0 gid=0\n\
             ./usr/bin/hello type=file mode=0755 size={} sha256digest={binary_hash} uid=0 gid=0\n\
             ./usr/bin/hello-real type=link link=hello uid=0 gid=0\n",
        binary.len()
    );
    write_mtree(&local_db, "hello", &mtree);
    local_db::write_file_list(
        &local_db,
        "hello",
        &[
            "usr/".to_string(),
            "usr/bin/".to_string(),
            "usr/bin/hello".to_string(),
            "usr/bin/hello-real".to_string(),
        ],
    )
    .expect("files");

    let report = check_package(&root, &local_db, "hello", true).expect("check");
    assert!(report.altered.is_empty(), "{report:?}");
    assert!(!report.metadata_missing);
    assert!(report.is_ok(true));

    write_file(&root, "usr/bin/hello", b"tampered", 0o755);
    let report = check_package(&root, &local_db, "hello", true).expect("check");
    assert_eq!(report.altered, vec!["usr/bin/hello".to_string()]);
    assert!(!report.is_ok(true));

    write_file(&root, "usr/bin/hello", binary, 0o644);
    let report = check_package(&root, &local_db, "hello", true).expect("check");
    assert_eq!(report.altered, vec!["usr/bin/hello".to_string()]);

    fs::remove_file(&target).expect("remove link");
    std::os::unix::fs::symlink("wrong-target", &target).expect("symlink");
    let report = check_package(&root, &local_db, "hello", true).expect("check");
    assert!(report.altered.contains(&"usr/bin/hello-real".to_string()));
}

#[test]
fn backup_changes_are_not_fatal() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let local_db = tmp.path().join("local");
    let pkg_dir = local_db.join("hello");
    fs::create_dir_all(&pkg_dir).expect("db");
    fs::write(pkg_dir.join("version"), "1.0-1").expect("version");

    write_file(&root, "etc/hello.conf", b"default", 0o644);
    let hash = sha256_hex(b"default");
    write_mtree(
        &local_db,
        "hello",
        &format!(
            "#mtree\n./etc/hello.conf type=file mode=0644 size=7 sha256digest={hash} uid=0 gid=0\n"
        ),
    );
    local_db::write_file_list(&local_db, "hello", &["etc/hello.conf".to_string()]).expect("files");
    local_db::write_backup_list(&local_db, "hello", &["etc/hello.conf".to_string()])
        .expect("backup");

    write_file(&root, "etc/hello.conf", b"edited by the user", 0o600);
    let report = check_package(&root, &local_db, "hello", true).expect("check");
    assert!(report.altered.is_empty(), "{report:?}");
    assert!(report.is_ok(true));
}

#[test]
fn missing_mtree_is_reported_in_full_mode_only() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let local_db = tmp.path().join("local");
    fs::create_dir_all(&local_db).expect("db");
    fs::create_dir_all(&root).expect("root");

    let fast = check_package(&root, &local_db, "legacy", false).expect("fast");
    assert_eq!(fast.total, 0);
    assert!(fast.is_ok(false));

    let full = check_package(&root, &local_db, "legacy", true).expect("full");
    assert!(full.metadata_missing);
    assert!(!full.is_ok(true));
}

#[test]
fn manifest_entries_without_mtree_records_are_metadata_issues() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let local_db = tmp.path().join("local");
    fs::create_dir_all(root.join("usr")).expect("dirs");
    write_file(&root, "usr/tracked", b"tracked", 0o644);
    write_file(&root, "usr/untracked", b"untracked", 0o644);
    let hash = sha256_hex(b"tracked");
    write_mtree(
        &local_db,
        "hello",
        &format!(
            "#mtree\n./usr/tracked type=file mode=0644 size=7 sha256digest={hash} uid=0 gid=0\n"
        ),
    );
    local_db::write_file_list(
        &local_db,
        "hello",
        &["usr/tracked".to_string(), "usr/untracked".to_string()],
    )
    .expect("files");

    let report = check_package(&root, &local_db, "hello", true).expect("check");
    assert_eq!(report.metadata_issues, vec!["usr/untracked".to_string()]);
    assert!(!report.is_ok(true));
}

#[test]
fn repair_rebuilds_files_from_mtree_and_preserves_shims() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");
    let pkg_dir = local_db.join("systemd");
    fs::create_dir_all(&pkg_dir).expect("db");
    fs::write(pkg_dir.join("version"), "262.1-1").expect("version");

    let empty_hash = sha256_hex(b"");
    write_mtree(
            &local_db,
            "systemd",
            &format!(
                "#mtree
                 ./usr type=dir mode=0755 uid=0 gid=0
                 ./usr/lib type=dir mode=0755 uid=0 gid=0
                 ./usr/lib/system-systemd\\134x2dcryptsetup.slice type=file mode=0644 size=0 sha256digest={empty_hash} uid=0 gid=0
"
            ),
        );
    local_db::write_file_list(
        &local_db,
        "systemd",
        &[
            "usr/".to_string(),
            "usr/lib/".to_string(),
            "usr/lib/system-systemd/x2dcryptsetup.slice".to_string(),
            "@ABS:/home/u/.local/bin/systemd-tool".to_string(),
        ],
    )
    .expect("files");

    let report = repair_package_manifest(&local_db, "systemd").expect("repair");
    assert!(report.repaired);
    assert!(!report.skipped);

    let files = local_db::read_files(&local_db, "systemd").expect("files");
    assert!(files.contains(&"usr/lib/system-systemd\\x2dcryptsetup.slice".to_string()));
    assert!(!files.contains(&"usr/lib/system-systemd/x2dcryptsetup.slice".to_string()));
    assert!(files.contains(&"@ABS:/home/u/.local/bin/systemd-tool".to_string()));

    let again = repair_package_manifest(&local_db, "systemd").expect("repair again");
    assert!(!again.repaired);
}

#[test]
fn missing_hashes_are_reported_as_metadata_issues() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let local_db = tmp.path().join("local");
    fs::create_dir_all(root.join("usr")).expect("dirs");
    write_file(&root, "usr/data", b"data", 0o644);
    write_mtree(
        &local_db,
        "hello",
        "#mtree\n./usr/data type=file mode=0644 size=4 uid=0 gid=0\n",
    );
    local_db::write_file_list(&local_db, "hello", &["usr/data".to_string()]).expect("files");

    let report = check_package(&root, &local_db, "hello", true).expect("check");
    assert_eq!(report.metadata_issues, vec!["usr/data".to_string()]);
    assert!(!report.is_ok(true));
}
