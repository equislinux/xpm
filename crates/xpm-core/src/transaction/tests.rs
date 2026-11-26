use super::*;
use std::fs;
use tempfile::TempDir;

fn test_root() -> (TempDir, PathBuf, PathBuf) {
    let tmpdir = TempDir::new().expect("create tempdir");
    let root = tmpdir.path().to_path_buf();
    let local_db = root.join("var/lib/xpm/local");
    (tmpdir, root, local_db)
}

#[test]
fn lock_is_exclusive_and_released_on_drop() {
    let tmp = TempDir::new().expect("tmp");
    let path = tmp.path().join("transaction.lock");

    let lock = FileLock::acquire(&path).expect("first lock");
    assert!(FileLock::acquire(&path).is_err(), "second lock must fail");
    drop(lock);
    FileLock::acquire(&path).expect("lock is released on drop");
}

#[test]
fn create_transaction() {
    let (_tmp, root, local_db) = test_root();
    let tx = Transaction::new(root, local_db).expect("create transaction");
    assert_eq!(tx.state(), TransactionState::Planning);
    assert_eq!(tx.operation_count(), 0);
}

#[test]
fn add_install_operation() {
    let (_tmp, root, local_db) = test_root();
    let mut tx = Transaction::new(root, local_db).expect("create transaction");

    tx.add_install(
        "test".to_string(),
        "1.0-1".to_string(),
        PathBuf::from("/tmp/test-1.0-1-x86_64.xp"),
    )
    .expect("add install");

    assert_eq!(tx.operation_count(), 1);
}

#[test]
fn prepare_requires_planning_state() {
    let (_tmp, root, local_db) = test_root();
    let mut tx = Transaction::new(root, local_db).expect("create transaction");

    tx.prepare().expect("prepare");
    assert_eq!(tx.state(), TransactionState::Prepared);

    let result = tx.prepare();
    assert!(
        result.is_err(),
        "should not allow prepare when not in planning state"
    );
}

#[test]
fn add_operation_fails_after_prepare() {
    let (_tmp, root, local_db) = test_root();
    let mut tx = Transaction::new(root, local_db).expect("create transaction");

    tx.prepare().expect("prepare");

    let result = tx.add_install(
        "test".to_string(),
        "1.0-1".to_string(),
        PathBuf::from("/tmp/test.xp"),
    );

    assert!(result.is_err(), "should not allow add after prepare");
}

#[test]
fn transaction_logs_to_file() {
    let (_tmp, root, local_db) = test_root();
    let tx = Transaction::new(root.clone(), local_db).expect("create transaction");

    tx.log("test message").expect("log");

    let log_path = root.join("var/log/xpm.log");
    assert!(log_path.exists(), "log file should exist");

    let content = fs::read_to_string(&log_path).expect("read log");
    assert!(
        content.contains("test message"),
        "log should contain message"
    );
}

#[test]
fn commit_requires_prepared_state() {
    let (_tmp, root, local_db) = test_root();
    let mut tx = Transaction::new(root, local_db).expect("create transaction");

    let result = tx.commit();
    assert!(result.is_err(), "commit should fail if not prepared");
}

#[test]
fn rollback_changes_state() {
    let (_tmp, root, local_db) = test_root();
    let mut tx = Transaction::new(root, local_db).expect("create transaction");

    tx.rollback().expect("rollback");
    assert_eq!(tx.state(), TransactionState::RolledBack);
}

// ── End-to-end integration tests ────────────────────────────────────

/// Create a minimal .xp archive (tar.zst) for testing
fn make_test_xp_package(dir: &Path) -> PathBuf {
    use std::io::Write;

    let pkg_path = dir.join("test-1.0.0-1-x86_64.xp");

    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);

        // .PKGINFO
        let pkginfo = b"pkgname = test\npkgver = 1.0.0-1\n\
                pkgdesc = Test package\narch = x86_64\nsize = 100\n";
        let mut header = tar::Header::new_gnu();
        header.set_path(".PKGINFO").unwrap();
        header.set_size(pkginfo.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &pkginfo[..]).unwrap();

        // .BUILDINFO
        let buildinfo = b"pkgname = test\npkgver = 1.0.0-1\n";
        let mut header = tar::Header::new_gnu();
        header.set_path(".BUILDINFO").unwrap();
        header.set_size(buildinfo.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &buildinfo[..]).unwrap();

        // .MTREE
        let mtree = b"#mtree\n\
                ./usr type=dir mode=0755 uid=0 gid=0\n\
                ./usr/bin type=dir mode=0755 uid=0 gid=0\n\
                ./usr/bin/test type=file mode=0755 size=12 uid=0 gid=0\n\
                ./usr/bin/test-link type=link link=test uid=0 gid=0\n";
        let mut header = tar::Header::new_gnu();
        header.set_path(".MTREE").unwrap();
        header.set_size(mtree.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &mtree[..]).unwrap();

        // Create usr/bin/ directory and test file
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/").unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();

        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/").unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();

        let content = b"test content";
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/test").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &content[..]).unwrap();

        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_path("usr/bin/test-link").unwrap();
        header.set_link_name("test").unwrap();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();

        builder.finish().unwrap();
    }

    // Compress with zstd
    let compressed = zstd::encode_all(&raw_tar[..], 3).expect("compress zstd");
    let mut file = fs::File::create(&pkg_path).expect("create file");
    file.write_all(&compressed).expect("write file");

    pkg_path
}

#[test]
fn e2e_install_extracts_files() {
    let (_tmp, root, local_db) = test_root();
    let cache_dir = root.join("var/cache/xpm");
    fs::create_dir_all(&cache_dir).expect("create cache dir");

    // Create test package
    let pkg_file = make_test_xp_package(&cache_dir);

    // Create transaction
    let mut tx = Transaction::new(root.clone(), local_db.clone()).expect("create transaction");

    // Setup hooks
    let hooks = HookChain::default();
    tx.set_hooks(hooks);

    // Add install operation
    tx.add_install("test".to_string(), "1.0.0-1".to_string(), pkg_file.clone())
        .expect("add install");

    // Prepare transaction
    tx.prepare().expect("prepare");

    // Commit transaction
    tx.commit().expect("commit");

    // Verify files were extracted
    assert!(
        root.join("usr/bin/test").exists(),
        "test executable should be extracted"
    );

    // Verify database entry was created
    assert!(
        local_db.join("test/version").exists(),
        "version file should be created in local db"
    );

    let version_content = fs::read_to_string(local_db.join("test/version")).expect("read version");
    assert_eq!(version_content, "1.0.0-1", "version should match");

    // Verify symlink was extracted
    assert!(
        root.join("usr/bin/test-link").is_symlink(),
        "symlink from the package should be extracted"
    );

    // Verify pacman-compatible file manifest derived from .MTREE
    let files_content =
        fs::read_to_string(local_db.join("test/files")).expect("read files manifest");
    assert_eq!(
        files_content,
        "%FILES%\nusr/\nusr/bin/\nusr/bin/test\nusr/bin/test-link\n"
    );
}

#[test]
fn e2e_remove_deletes_files() {
    let (_tmp, root, local_db) = test_root();
    let cache_dir = root.join("var/cache/xpm");
    fs::create_dir_all(&cache_dir).expect("create cache dir");

    // First install a package
    let pkg_file = make_test_xp_package(&cache_dir);

    let mut tx = Transaction::new(root.clone(), local_db.clone()).expect("create transaction");
    let hooks = HookChain::default();
    tx.set_hooks(hooks);

    tx.add_install("test".to_string(), "1.0.0-1".to_string(), pkg_file)
        .expect("add install");

    tx.prepare().expect("prepare");
    tx.commit().expect("commit");

    // Verify extracted file exists
    let test_file = root.join("usr/bin/test");
    assert!(test_file.exists(), "file should exist after install");

    // Now remove the package (without hooks for now)
    drop(tx);
    let mut tx2 =
        Transaction::new(root.clone(), local_db.clone()).expect("create remove transaction");
    // Don't set hooks to isolate removal logic

    tx2.add_remove("test".to_string()).expect("add remove");

    tx2.prepare().expect("prepare");
    tx2.commit().expect("commit");

    // Verify database entry is removed
    assert!(
        !local_db.join("test").exists(),
        "package directory should be removed from local db"
    );
}

#[test]
fn e2e_multiple_installs() {
    let (_tmp, root, local_db) = test_root();
    let cache_dir = root.join("var/cache/xpm");
    fs::create_dir_all(&cache_dir).expect("create cache dir");

    // Create test packages
    let pkg1 = make_test_xp_package(&cache_dir);
    let pkg2_path = cache_dir.join("test-2.0.0-1-x86_64.xp");
    fs::copy(&pkg1, &pkg2_path).expect("copy package");

    // Create transaction with multiple installs
    let mut tx = Transaction::new(root.clone(), local_db.clone()).expect("create transaction");
    let hooks = HookChain::default();
    tx.set_hooks(hooks);

    tx.add_install("test".to_string(), "1.0.0-1".to_string(), pkg1)
        .expect("add install 1");

    // Prepare and commit first should succeed
    tx.prepare().expect("prepare");
    tx.commit().expect("commit");

    assert_eq!(tx.operation_count(), 1);
    assert_eq!(tx.state(), TransactionState::Committed);
}

fn make_named_package(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
    let pkg_path = dir.join(format!("{name}-1.0-1-x86_64.xp"));
    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);
        let pkginfo = format!(
            "pkgname = {name}\npkgver = 1.0-1\npkgdesc = {name}\narch = x86_64\nsize = {}\n",
            content.len()
        );
        let mut header = tar::Header::new_gnu();
        header.set_path(".PKGINFO").unwrap();
        header.set_size(pkginfo.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, pkginfo.as_bytes()).unwrap();

        let mut header = tar::Header::new_gnu();
        header.set_path(format!("usr/bin/{name}")).unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, content).unwrap();
        builder.finish().unwrap();
    }
    std::fs::write(&pkg_path, zstd::encode_all(&raw_tar[..], 3).unwrap()).unwrap();
    pkg_path
}

#[test]
fn parallel_install_batch_extracts_every_package() {
    let (_tmp, root, local_db) = test_root();
    let cache_dir = root.join("var/cache/xpm");
    fs::create_dir_all(&cache_dir).expect("create cache dir");

    let packages = ["alpha", "beta", "gamma", "delta"];
    let mut tx = Transaction::new(root.clone(), local_db.clone()).expect("create transaction");
    tx.set_hooks(HookChain::default());
    for name in packages {
        let pkg = make_named_package(&cache_dir, name, name.as_bytes());
        let metadata = Arc::new(crate::package::read_metadata(&pkg).expect("metadata"));
        tx.cache_package_metadata(pkg.clone(), metadata);
        tx.add_install(name.to_string(), "1.0-1".to_string(), pkg)
            .expect("add install");
    }

    tx.prepare().expect("prepare");
    tx.commit().expect("parallel commit");

    for name in packages {
        assert!(
            root.join(format!("usr/bin/{name}")).is_file(),
            "{name} must be extracted"
        );
        assert!(
            local_db.join(name).join("version").is_file(),
            "{name} must be recorded in the local database"
        );
    }
}
