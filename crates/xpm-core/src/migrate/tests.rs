use super::*;
use tempfile::TempDir;

fn write_pacman_entry(root: &Path, name: &str, version: &str, desc: &str, files: &str) {
    let dir = root.join(format!("{name}-{version}"));
    fs::create_dir_all(&dir).expect("create entry");
    fs::write(dir.join("desc"), desc).expect("desc");
    fs::write(dir.join("files"), files).expect("files");
}

fn desc(name: &str, version: &str, reason: &str) -> String {
    format!(
            "%NAME%\n{name}\n\n%VERSION%\n{version}\n\n%REASON%\n{reason}\n\n%DEPENDS%\nlibfoo\nbar>=2\n\n%PROVIDES%\nvirtual={version}\n"
        )
}

#[test]
fn imports_desc_files_backups_and_reason() {
    let tmp = TempDir::new().expect("tmp");
    let pacman = tmp.path().join("pacman");
    let dest = tmp.path().join("xpm");
    write_pacman_entry(
        &pacman,
        "foo",
        "1.2-3",
        &desc("foo", "1.2-3", "1"),
        "%FILES%\nusr/\nusr/bin/foo\netc/foo.conf\n\n%BACKUP%\netc/foo.conf\tdeadbeef\n",
    );

    let report = import_pacman_local(&pacman, &dest, false).expect("import");
    assert_eq!(report.imported, 1);
    assert_eq!(report.skipped, 0);
    assert!(report.failed.is_empty());

    assert_eq!(
        local_db::read_version(&dest, "foo").as_deref(),
        Some("1.2-3")
    );
    assert_eq!(InstallReason::read(&dest, "foo"), InstallReason::Dep);
    assert_eq!(
        local_db::read_depends(&dest, "foo").expect("depends"),
        Some(vec!["libfoo".to_string(), "bar>=2".to_string()])
    );
    assert_eq!(
        local_db::read_provides(&dest, "foo").expect("provides"),
        vec!["virtual=1.2-3".to_string()]
    );
    assert_eq!(
        local_db::read_backup_list(&dest, "foo").expect("backup"),
        vec!["etc/foo.conf".to_string()]
    );
    let files = local_db::read_files(&dest, "foo").expect("files");
    assert_eq!(
        files,
        vec![
            "usr/".to_string(),
            "usr/bin/foo".to_string(),
            "etc/foo.conf".to_string()
        ]
    );
}

#[test]
fn import_prefers_mtree_over_mangled_files() {
    let tmp = TempDir::new().expect("tmp");
    let pacman = tmp.path().join("pacman");
    let dest = tmp.path().join("xpm");
    write_pacman_entry(
        &pacman,
        "systemd",
        "262.1-1",
        &desc("systemd", "262.1-1", "0"),
        "%FILES%\nusr/lib/systemd/system-systemd/x2dcryptsetup.slice\n",
    );
    fs::write(
            pacman.join("systemd-262.1-1").join("mtree"),
            "#mtree\n\
             ./usr type=dir mode=0755 uid=0 gid=0\n\
             ./usr/lib type=dir mode=0755 uid=0 gid=0\n\
             ./usr/lib/system-systemd\\134x2dcryptsetup.slice type=file mode=0644 size=0 sha256digest=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855 uid=0 gid=0\n",
        )
        .expect("mtree");

    import_pacman_local(&pacman, &dest, false).expect("import");
    let files = local_db::read_files(&dest, "systemd").expect("files");
    assert!(
        files.contains(&"usr/lib/system-systemd\\x2dcryptsetup.slice".to_string()),
        "{files:?}"
    );
    assert!(
        !files.contains(&"usr/lib/systemd/system-systemd/x2dcryptsetup.slice".to_string()),
        "{files:?}"
    );
}

#[test]
fn skips_existing_entries_unless_forced() {
    let tmp = TempDir::new().expect("tmp");
    let pacman = tmp.path().join("pacman");
    let dest = tmp.path().join("xpm");
    write_pacman_entry(
        &pacman,
        "foo",
        "1.0-1",
        &desc("foo", "1.0-1", "0"),
        "%FILES%\nusr/bin/foo\n",
    );

    assert_eq!(
        import_pacman_local(&pacman, &dest, false)
            .expect("first")
            .imported,
        1
    );
    assert_eq!(
        import_pacman_local(&pacman, &dest, false)
            .expect("second")
            .skipped,
        1
    );

    // Simulate a pacman upgrade: the old entry directory is replaced.
    fs::remove_dir_all(pacman.join("foo-1.0-1")).expect("remove old entry");
    write_pacman_entry(
        &pacman,
        "foo",
        "2.0-1",
        &desc("foo", "2.0-1", "0"),
        "%FILES%\nusr/bin/foo\n",
    );
    let report = import_pacman_local(&pacman, &dest, true).expect("force");
    assert_eq!(report.imported, 1);
    assert_eq!(
        local_db::read_version(&dest, "foo").as_deref(),
        Some("2.0-1")
    );
}

#[test]
fn refuses_to_import_into_the_source_database() {
    let tmp = TempDir::new().expect("tmp");
    let pacman = tmp.path().join("pacman");
    write_pacman_entry(
        &pacman,
        "foo",
        "1.0-1",
        &desc("foo", "1.0-1", "0"),
        "%FILES%\nusr/bin/foo\n",
    );

    let err = import_pacman_local(&pacman, &pacman, false).expect_err("must refuse");
    assert!(format!("{err}").contains("itself"));
}
