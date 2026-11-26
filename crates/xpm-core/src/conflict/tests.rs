use super::*;
use std::os::unix::fs::symlink;
use tempfile::TempDir;

fn target(name: &str, files: &[&str]) -> TargetManifest {
    TargetManifest {
        name: name.to_string(),
        files: files.iter().map(|file| file.to_string()).collect(),
        backups: Vec::new(),
        old_files: Vec::new(),
    }
}

fn installed(name: &str, files: &[&str]) -> InstalledManifest {
    InstalledManifest {
        name: name.to_string(),
        files: files.iter().map(|file| file.to_string()).collect(),
        backups: Vec::new(),
    }
}

fn overwrite(patterns: &[&str]) -> OverwritePatterns {
    OverwritePatterns::parse(patterns.iter().copied())
}

fn write_file(root: &Path, relative: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, "existing").expect("write file");
}

#[test]
fn unowned_file_conflicts() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/probe");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["usr/bin/probe"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].target, "new");
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
    assert_eq!(conflicts[0].owner, None);
    assert_eq!(conflicts[0].path, rooted(temp.path(), "usr/bin/probe"));
}

#[test]
fn owned_file_conflicts() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/probe");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[installed("owner", &["usr/bin/probe"])],
        &[target("new", &["usr/bin/probe"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemOwned);
    assert_eq!(conflicts[0].owner.as_deref(), Some("owner"));
}

#[test]
fn target_against_target_conflict() {
    let temp = TempDir::new().expect("tempdir");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[
            target("owner", &["usr/bin/probe"]),
            target("new", &["usr/bin/probe"]),
        ],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::TargetTarget);
    assert_eq!(conflicts[0].target, "owner");
    assert_eq!(conflicts[0].other_target.as_deref(), Some("new"));
}

#[test]
fn shared_directories_are_not_conflicts() {
    let temp = TempDir::new().expect("tempdir");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[
            target("shared-a", &["usr/share/common/"]),
            target("shared-b", &["usr/share/common/"]),
        ],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert!(conflicts.is_empty());
}

#[test]
fn target_directory_against_target_file_is_not_forced() {
    let temp = TempDir::new().expect("tempdir");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("dirpkg", &["foo/"]), target("filepkg", &["foo"])],
        &overwrite(&["foo", "foo/"]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::TargetTarget);
}

#[test]
fn dangling_symlink_counts_as_existing_file() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("usr/bin/probe");
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    symlink("missing", &path).expect("symlink");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["usr/bin/probe"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
    assert!(fs::symlink_metadata(&path)
        .expect("lstat")
        .file_type()
        .is_symlink());
}

#[test]
fn unowned_backup_file_is_not_a_conflict() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "etc/test.conf");

    let mut incoming = target("conf", &["etc/test.conf"]);
    incoming.backups.push("etc/test.conf".to_string());

    let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
        .expect("check conflicts");

    assert!(conflicts.is_empty());
}

#[test]
fn overwrite_pattern_suppresses_file_conflict() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/probe");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["usr/bin/probe"])],
        &overwrite(&["usr/bin/*"]),
    )
    .expect("check conflicts");

    assert!(conflicts.is_empty());
}

#[test]
fn negated_overwrite_pattern_wins_last() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/probe");

    let patterns = overwrite(&["usr/bin/probe", "!usr/bin/probe"]);
    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["usr/bin/probe"])],
        &patterns,
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
}

#[test]
fn overwrite_matches_rooted_path() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/probe");
    let pattern = rooted(temp.path(), "usr/bin/probe");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["usr/bin/probe"])],
        &overwrite(&[&pattern]),
    )
    .expect("check conflicts");

    assert!(conflicts.is_empty());
}

#[test]
fn overwrite_patterns_split_commas_and_ignore_empty() {
    let patterns = OverwritePatterns::parse(["", "usr/*,!usr/bin/probe", "foo"]);
    assert!(!patterns.is_empty());
    assert!(patterns.matches("usr/share/x", "/root/usr/share/x"));
    assert!(!patterns.matches("usr/bin/probe", "/root/usr/bin/probe"));
    assert!(patterns.matches("foo", "/root/foo"));
    assert!(OverwritePatterns::parse(["", ","]).is_empty());
}

#[test]
fn overwrite_escapes_leading_bang() {
    let patterns = overwrite(&["\\!literal"]);
    assert!(patterns.matches("!literal", "/root/!literal"));
    assert!(!patterns.matches("literal", "/root/literal"));
}

#[test]
fn upgrade_only_checks_new_files() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/probe");
    write_file(temp.path(), "usr/bin/added");

    let mut incoming = target("pkg", &["usr/bin/probe", "usr/bin/added"]);
    incoming.old_files.push("usr/bin/probe".to_string());

    let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
        .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, rooted(temp.path(), "usr/bin/added"));
}

#[test]
fn imported_owner_is_reported_by_name() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/probe");

    let imported = installed("imported", &["usr/bin/probe"]);
    let conflicts = check_file_conflicts(
        temp.path(),
        &[imported],
        &[target("new", &["usr/bin/probe"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].owner.as_deref(), Some("imported"));
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemOwned);
}

#[test]
fn file_transferred_from_upgraded_package_is_not_a_conflict() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/lib/libcrypto.so.3");

    let old = installed("openssl", &["usr/lib/libcrypto.so.3"]);
    let mut upgrade = target("openssl", &[]);
    upgrade.old_files.push("usr/lib/libcrypto.so.3".to_string());
    let compat = target("openssl-3", &["usr/lib/libcrypto.so.3"]);
    let removed = HashSet::from(["openssl".to_string()]);

    let conflicts = check_file_conflicts_with_removals(
        temp.path(),
        &[old],
        &[upgrade, compat],
        &removed,
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert!(conflicts.is_empty(), "{conflicts:?}");
}

#[test]
fn file_transfer_does_not_hide_a_remaining_owner() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/lib/libshared.so");

    let leaving = installed("leaving", &["usr/lib/libshared.so"]);
    let remaining = installed("remaining", &["usr/lib/libshared.so"]);
    let incoming = target("new", &["usr/lib/libshared.so"]);
    let removed = HashSet::from(["leaving".to_string()]);

    let conflicts = check_file_conflicts_with_removals(
        temp.path(),
        &[leaving, remaining],
        &[incoming],
        &removed,
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemOwned);
    assert_eq!(conflicts[0].owner.as_deref(), Some("remaining"));
}

#[test]
fn file_still_shipped_by_upgraded_owner_remains_target_conflict() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/lib/libboth.so");

    let old = installed("owner", &["usr/lib/libboth.so"]);
    let mut upgrade = target("owner", &["usr/lib/libboth.so"]);
    upgrade.old_files.push("usr/lib/libboth.so".to_string());
    let incoming = target("new", &["usr/lib/libboth.so"]);
    let removed = HashSet::from(["owner".to_string()]);

    let conflicts = check_file_conflicts_with_removals(
        temp.path(),
        &[old],
        &[upgrade, incoming],
        &removed,
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::TargetTarget);
}

#[test]
fn backslash_paths_are_ordinary_bytes() {
    let temp = TempDir::new().expect("tempdir");
    let path = "usr/lib/systemd/system/system-systemd\\x2dcryptsetup.slice";
    write_file(temp.path(), path);

    let old = installed("systemd", &[path]);
    let mut upgrade = target("systemd", &[]);
    upgrade.old_files.push(path.to_string());
    let incoming = target("systemd-slices", &[path]);
    let removed = HashSet::from(["systemd".to_string()]);

    let conflicts = check_file_conflicts_with_removals(
        temp.path(),
        &[old],
        &[upgrade, incoming],
        &removed,
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert!(conflicts.is_empty(), "{conflicts:?}");
}

#[test]
fn empty_file_manifest_does_not_provide_ownership() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "usr/bin/virt");

    let virtual_package = installed("virtual-b", &[]);
    let conflicts = check_file_conflicts(
        temp.path(),
        &[virtual_package],
        &[target("new", &["usr/bin/virt"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
    assert_eq!(conflicts[0].owner, None);
}

#[test]
fn file_cannot_replace_unowned_directory_even_with_overwrite() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "foo/child");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["foo"])],
        &overwrite(&["foo"]),
    )
    .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].kind, ConflictKind::FilesystemUnowned);
    assert_eq!(conflicts[0].path, rooted(temp.path(), "foo"));
}

#[test]
fn directory_can_replace_file_with_overwrite() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "foo");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["foo/"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");
    assert_eq!(conflicts.len(), 1);

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["foo/"])],
        &overwrite(&["foo"]),
    )
    .expect("check conflicts");
    assert!(conflicts.is_empty());
}

#[test]
fn directory_tree_owned_by_old_package_can_be_replaced() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "foo/x");

    let mut incoming = target("pkg", &["foo"]);
    incoming.old_files.push("foo/".to_string());
    incoming.old_files.push("foo/x".to_string());

    let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
        .expect("check conflicts");

    assert!(conflicts.is_empty());
}

#[test]
fn directory_tree_with_unowned_child_conflicts() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "foo/x");
    write_file(temp.path(), "foo/y");

    let mut incoming = target("pkg", &["foo"]);
    incoming.old_files.push("foo/".to_string());
    incoming.old_files.push("foo/x".to_string());

    let conflicts = check_file_conflicts(temp.path(), &[], &[incoming], &overwrite(&[]))
        .expect("check conflicts");

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].path, rooted(temp.path(), "foo"));
}

#[test]
fn unsafe_target_paths_are_rejected() {
    let temp = TempDir::new().expect("tempdir");

    for path in ["/etc/passwd", "../escape", "usr/../../escape"] {
        let result =
            check_file_conflicts(temp.path(), &[], &[target("bad", &[path])], &overwrite(&[]));
        assert!(matches!(result, Err(XpmError::Package(_))));
    }
}

#[test]
fn leading_dot_slash_is_stripped_and_duplicates_removed() {
    let temp = TempDir::new().expect("tempdir");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("pkg", &["./foo", "./foo", "", ".", "./", "/"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    assert!(conflicts.is_empty());
}

#[test]
fn conflicts_are_sorted_by_path_then_target() {
    let temp = TempDir::new().expect("tempdir");
    write_file(temp.path(), "z");
    write_file(temp.path(), "a");

    let conflicts = check_file_conflicts(
        temp.path(),
        &[],
        &[target("new", &["z", "a"])],
        &overwrite(&[]),
    )
    .expect("check conflicts");

    let paths: Vec<&str> = conflicts
        .iter()
        .map(|conflict| conflict.path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![rooted(temp.path(), "a"), rooted(temp.path(), "z"),]
    );
}
