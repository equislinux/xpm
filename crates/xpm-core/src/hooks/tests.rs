use super::*;
use tempfile::TempDir;

fn test_context(op_type: OperationType) -> (TempDir, TempDir, HookContext) {
    let root_tmp = TempDir::new().expect("create root tempdir");
    let db_tmp = TempDir::new().expect("create db tempdir");

    let ctx = HookContext {
        operation_type: op_type,
        pkg_name: "test".to_string(),
        pkg_version: "1.0-1".to_string(),
        old_version: None,
        pkg_file: None,
        root_dir: root_tmp.path().to_path_buf(),
        local_db_dir: db_tmp.path().to_path_buf(),
        save_configs: true,
        shell_integration: false,
        scriptlet_mode: ScriptletMode::Auto,
        metadata: None,
        transaction_paths: None,
    };

    (root_tmp, db_tmp, ctx)
}

#[test]
fn hook_chain_default_has_hooks() {
    let chain = HookChain::default();
    assert!(!chain.hooks().is_empty());
    assert_eq!(chain.hooks().len(), 6); // metadata, pre-scriptlet, extraction, removal, post-scriptlet, localdb
}

#[test]
fn post_scriptlet_hook_runs_post_install() {
    let (_root, db_tmp, mut ctx) = test_context(OperationType::Install);
    let hook = PostScriptletHook;
    ctx.pkg_file = None;
    ctx.root_dir = PathBuf::from("/");

    let marker = std::env::temp_dir().join(format!(
        "xpm-scriptlet-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let pkg_dir = db_tmp.path().join(&ctx.pkg_name);
    fs::create_dir_all(&pkg_dir).expect("create pkg dir");
    fs::write(
        pkg_dir.join("install"),
        format!("post_install() {{ touch '{}'; }}\n", marker.display()),
    )
    .expect("write install script");

    hook.run(&ctx).expect("run scriptlet hook");
    assert!(marker.exists(), "post_install should create marker file");
    let _ = fs::remove_file(marker);
}

#[test]
fn scriptlet_modes_handle_missing_target_shell() {
    let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
    ctx.pkg_file = None;

    let pkg_dir = db_tmp.path().join(&ctx.pkg_name);
    fs::create_dir_all(&pkg_dir).expect("pkg dir");
    fs::write(
        pkg_dir.join("install"),
        "post_install() { touch /should-not-run; }\n",
    )
    .expect("install script");

    ctx.scriptlet_mode = ScriptletMode::Auto;
    PostScriptletHook
        .run(&ctx)
        .expect("auto skips without shell");
    assert!(!root.path().join("should-not-run").exists());

    ctx.scriptlet_mode = ScriptletMode::Require;
    let err = PostScriptletHook
        .run(&ctx)
        .expect_err("require fails without shell");
    assert!(format!("{err}").contains("/bin/sh"));

    ctx.scriptlet_mode = ScriptletMode::Skip;
    PostScriptletHook.run(&ctx).expect("skip never runs");
    assert!(!root.path().join("should-not-run").exists());
}

#[test]
fn missing_scriptlet_function_does_not_require_a_shell() {
    let (_root, db_tmp, mut ctx) = test_context(OperationType::Install);
    ctx.pkg_file = None;
    ctx.scriptlet_mode = ScriptletMode::Require;

    let pkg_dir = db_tmp.path().join(&ctx.pkg_name);
    fs::create_dir_all(&pkg_dir).expect("pkg dir");
    fs::write(
        pkg_dir.join("install"),
        "post_install() { touch /never; }\n",
    )
    .expect("install script");

    PreScriptletHook
        .run(&ctx)
        .expect("pre_install is absent, so no shell is required");
}

#[test]
fn normalize_archive_path_preserves_dotfiles_and_backslashes() {
    assert_eq!(
        normalize_archive_path(Path::new("./usr/bin/foo")),
        "usr/bin/foo"
    );
    assert_eq!(normalize_archive_path(Path::new("./.hidden")), ".hidden");
    assert_eq!(
        normalize_archive_path(Path::new("usr/bin/a\\b")),
        "usr/bin/a\\b"
    );
    assert!(!is_metadata_path("usr/share/foo.PKGINFO"));
    assert!(is_metadata_path(".PKGINFO"));
}

#[test]
fn extraction_rejects_symlink_ancestors() {
    let root = TempDir::new().expect("root");
    fs::create_dir_all(root.path().join("usr")).expect("usr");
    std::os::unix::fs::symlink("/tmp", root.path().join("usr/lib")).expect("symlink");

    let err = ensure_no_symlink_ancestors(root.path(), &root.path().join("usr/lib/foo"))
        .expect_err("must reject symlink ancestor");
    assert!(format!("{err}").contains("symlink"));

    ensure_no_symlink_ancestors(root.path(), &root.path().join("usr/bin/foo"))
        .expect("regular ancestors are fine");
}

#[test]
fn local_db_hook_creates_version_file() {
    let (_root, _db_tmp, ctx) = test_context(OperationType::Install);
    let hook = LocalDbHook;

    hook.run(&ctx).expect("run hook");

    let version_file = ctx.local_db_dir.join(&ctx.pkg_name).join("version");
    assert!(version_file.exists());

    let content = fs::read_to_string(&version_file).expect("read version file");
    assert_eq!(content, "1.0-1");
}

#[test]
fn local_db_hook_remove_deletes_entry() {
    let (_root, _db_tmp, ctx) = test_context(OperationType::Remove);
    let pkg_dir = ctx.local_db_dir.join(&ctx.pkg_name);

    // Create the entry first
    fs::create_dir_all(&pkg_dir).expect("create pkg dir");
    fs::write(pkg_dir.join("version"), "1.0-1").expect("write version");

    // Now remove it
    let hook = LocalDbHook;
    hook.run(&ctx).expect("run hook");

    assert!(!pkg_dir.exists(), "pkg directory should be deleted");
}

#[test]
fn file_removal_hook_skips_if_no_files() {
    let (_root, _db_tmp, ctx) = test_context(OperationType::Remove);
    let hook = FileRemovalHook;

    // Should not error even if no file list exists
    let result = hook.run(&ctx);
    assert!(result.is_ok());
}

#[test]
fn file_removal_hook_removes_files_and_prunes_empty_dirs() {
    let (root, _db_tmp, ctx) = test_context(OperationType::Remove);
    let hook = FileRemovalHook;

    let installed = root.path().join("usr/bin/xfetch");
    fs::create_dir_all(installed.parent().expect("parent dir")).expect("create parent dirs");
    fs::write(&installed, "binary").expect("write binary file");
    // A package-owned symlink must be removed as well (roadmap #24).
    let link = root.path().join("usr/bin/xfetch-link");
    std::os::unix::fs::symlink("xfetch", &link).expect("create symlink");

    let pkg_dir = ctx.local_db_dir.join(&ctx.pkg_name);
    fs::create_dir_all(&pkg_dir).expect("create package db dir");
    fs::write(
        pkg_dir.join("files"),
        "%FILES%\nusr/\nusr/bin/\nusr/bin/xfetch\nusr/bin/xfetch-link\n",
    )
    .expect("write files manifest");

    hook.run(&ctx).expect("run hook");

    assert!(!installed.exists(), "installed file should be removed");
    assert!(!link.exists(), "package-owned symlink should be removed");
    assert!(
        !root.path().join("usr/bin").exists(),
        "empty bin directory should be pruned"
    );
}

#[test]
fn hook_chain_runs_all_hooks() {
    let (_root, _db_tmp, ctx) = test_context(OperationType::Install);
    let mut chain = HookChain::new();
    chain.add_hook(Box::new(LocalDbHook));

    let result = chain.run(&ctx);
    assert!(result.is_ok());
}

// ── Configuration-file handling (.pacnew / .pacsave) ────────────────

use sha2::{Digest, Sha256};

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Builds a minimal `.xp` with payload files, a matching `.MTREE` and a
/// `.PKGINFO` carrying the `backup` list. `version` is `1.0-1` style.
fn write_config_package(
    dir: &Path,
    name: &str,
    version: &str,
    files: &[(&str, &[u8])],
    backup: &[&str],
) -> PathBuf {
    let path = dir.join(format!("{name}-{version}-x86_64.xp"));

    let mut pkginfo =
        format!("pkgname = {name}\npkgver = {version}\npkgdesc = test\narch = x86_64\n");
    for entry in backup {
        pkginfo.push_str(&format!("backup = {entry}\n"));
    }

    let mut mtree = String::from("#mtree\n");
    for (file, content) in files {
        mtree.push_str(&format!(
            "./{file} type=file mode=0644 size={} sha256digest={} uid=0 gid=0\n",
            content.len(),
            sha256_hex(content)
        ));
    }

    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);
        for (entry, data) in [
            (".PKGINFO", pkginfo.as_bytes()),
            (".MTREE", mtree.as_bytes()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_path(entry).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, data).unwrap();
        }
        for (file, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_path(file).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, *content).unwrap();
        }
        builder.finish().unwrap();
    }

    let compressed = zstd::encode_all(&raw_tar[..], 1).expect("compress");
    fs::write(&path, compressed).expect("write package");
    path
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn write_setuid_package(dir: &Path) -> PathBuf {
    let path = dir.join("setuid-1.0-1-x86_64.xp");
    let pkginfo = "pkgname = setuid\npkgver = 1.0-1\npkgdesc = test\narch = x86_64\n";
    let content = b"#!/bin/sh\n";
    let mtree = format!(
        "#mtree\n./usr/bin/setuid type=file mode=04755 size={} sha256digest={} uid=0 gid=0\n",
        content.len(),
        sha256_hex(content)
    );
    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);
        for (entry, data) in [
            (".PKGINFO", pkginfo.as_bytes()),
            (".MTREE", mtree.as_bytes()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_path(entry).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, data).unwrap();
        }
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/setuid").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o4755);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &content[..]).unwrap();
        builder.finish().unwrap();
    }
    fs::write(&path, zstd::encode_all(&raw_tar[..], 1).unwrap()).unwrap();
    path
}

#[test]
fn extraction_preserves_setuid_mode() {
    use std::os::unix::fs::PermissionsExt;

    let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
    ctx.pkg_name = "setuid".to_string();
    ctx.pkg_file = Some(write_setuid_package(db_tmp.path()));

    FileExtractionHook.run(&ctx).expect("extract");
    let target = root.path().join("usr/bin/setuid");
    let mode = fs::metadata(&target)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o7777;
    assert_eq!(mode, 0o4755, "setuid bit must be preserved");
}

#[test]
fn install_keeps_existing_config_and_writes_pacnew() {
    let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
    fs::create_dir_all(root.path().join("etc")).expect("etc dir");
    fs::write(root.path().join("etc/foo.conf"), "user content").expect("existing config");

    let pkg = write_config_package(
        db_tmp.path(),
        "hello",
        "1.0-1",
        &[("etc/foo.conf", b"package content")],
        &["etc/foo.conf"],
    );
    ctx.pkg_name = "hello".to_string();
    ctx.pkg_file = Some(pkg);

    FileExtractionHook.run(&ctx).expect("extract");

    assert_eq!(read(&root.path().join("etc/foo.conf")), "user content");
    assert_eq!(
        read(&root.path().join("etc/foo.conf.pacnew")),
        "package content"
    );
    // The new version is not owned by the package manifest.
    let files = crate::local_db::read_files(db_tmp.path(), "hello").expect("files");
    assert!(!files.iter().any(|f| f.contains("pacnew")));
    // The backup list and .MTREE are recorded for the next transaction.
    assert_eq!(
        crate::local_db::read_backup_list(db_tmp.path(), "hello").expect("backup"),
        vec!["etc/foo.conf".to_string()]
    );
    assert!(crate::local_db::read_mtree(db_tmp.path(), "hello")
        .expect("mtree")
        .is_some());
}

#[test]
fn install_identical_config_overwrites_without_pacnew() {
    let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
    fs::create_dir_all(root.path().join("etc")).expect("etc dir");
    fs::write(root.path().join("etc/foo.conf"), "same").expect("existing config");

    let pkg = write_config_package(
        db_tmp.path(),
        "hello",
        "1.0-1",
        &[("etc/foo.conf", b"same")],
        &["etc/foo.conf"],
    );
    ctx.pkg_file = Some(pkg);

    FileExtractionHook.run(&ctx).expect("extract");

    assert_eq!(read(&root.path().join("etc/foo.conf")), "same");
    assert!(!root.path().join("etc/foo.conf.pacnew").exists());
}

/// Installs v1 through the real hook so the local database gets the
/// manifest, backup list and hash map that the upgrade will compare with.
fn install_v1(root: &Path, db: &Path, version: &str, files: &[(&str, &[u8])], backup: &[&str]) {
    let pkg = write_config_package(db, "hello", version, files, backup);
    let ctx = HookContext {
        operation_type: OperationType::Install,
        pkg_name: "hello".to_string(),
        pkg_version: version.to_string(),
        old_version: None,
        pkg_file: Some(pkg),
        root_dir: root.to_path_buf(),
        local_db_dir: db.to_path_buf(),
        save_configs: true,
        shell_integration: false,
        scriptlet_mode: ScriptletMode::Auto,
        metadata: None,
        transaction_paths: None,
    };
    FileExtractionHook.run(&ctx).expect("install v1");
}

fn upgrade_context(root: &Path, db: &Path, pkg: PathBuf, from: &str, to: &str) -> HookContext {
    HookContext {
        operation_type: OperationType::Upgrade,
        pkg_name: "hello".to_string(),
        pkg_version: to.to_string(),
        old_version: Some(from.to_string()),
        pkg_file: Some(pkg),
        root_dir: root.to_path_buf(),
        local_db_dir: db.to_path_buf(),
        save_configs: true,
        shell_integration: false,
        scriptlet_mode: ScriptletMode::Auto,
        metadata: None,
        transaction_paths: None,
    }
}

#[test]
fn upgrade_replaces_unmodified_config() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let db = tmp.path().join("db");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&db).expect("db");

    install_v1(
        &root,
        &db,
        "1.0-1",
        &[("etc/foo.conf", b"v1")],
        &["etc/foo.conf"],
    );
    assert_eq!(read(&root.join("etc/foo.conf")), "v1");

    let pkg = write_config_package(
        &db,
        "hello",
        "2.0-1",
        &[("etc/foo.conf", b"v2")],
        &["etc/foo.conf"],
    );
    let ctx = upgrade_context(&root, &db, pkg, "1.0-1", "2.0-1");
    FileExtractionHook.run(&ctx).expect("upgrade");

    assert_eq!(read(&root.join("etc/foo.conf")), "v2");
    assert!(!root.join("etc/foo.conf.pacnew").exists());
}

#[test]
fn upgrade_keeps_modified_config_and_writes_pacnew() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let db = tmp.path().join("db");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&db).expect("db");

    install_v1(
        &root,
        &db,
        "1.0-1",
        &[("etc/foo.conf", b"v1")],
        &["etc/foo.conf"],
    );
    fs::write(root.join("etc/foo.conf"), "user edited").expect("edit config");

    let pkg = write_config_package(
        &db,
        "hello",
        "2.0-1",
        &[("etc/foo.conf", b"v2")],
        &["etc/foo.conf"],
    );
    let ctx = upgrade_context(&root, &db, pkg, "1.0-1", "2.0-1");
    FileExtractionHook.run(&ctx).expect("upgrade");

    assert_eq!(read(&root.join("etc/foo.conf")), "user edited");
    assert_eq!(read(&root.join("etc/foo.conf.pacnew")), "v2");
}

#[test]
fn upgrade_removes_stale_files_and_pacsaves_modified_config() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let db = tmp.path().join("db");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&db).expect("db");

    install_v1(
        &root,
        &db,
        "1.0-1",
        &[
            ("etc/old.conf", b"v1 config"),
            ("usr/share/stale.txt", b"stale"),
        ],
        &["etc/old.conf"],
    );
    fs::write(root.join("etc/old.conf"), "user edited").expect("edit config");

    let pkg = write_config_package(&db, "hello", "2.0-1", &[("usr/share/new.txt", b"new")], &[]);
    let ctx = upgrade_context(&root, &db, pkg, "1.0-1", "2.0-1");
    FileExtractionHook.run(&ctx).expect("upgrade");

    // Modified config dropped by v2 is preserved as .pacsave.
    assert!(!root.join("etc/old.conf").exists());
    assert_eq!(read(&root.join("etc/old.conf.pacsave")), "user edited");
    // Plain stale file is gone.
    assert!(!root.join("usr/share/stale.txt").exists());
    assert!(root.join("usr/share/new.txt").exists());
}

#[test]
fn upgrade_keeps_file_taken_over_by_another_target() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let db = tmp.path().join("db");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&db).expect("db");

    install_v1(
        &root,
        &db,
        "1.0-1",
        &[("usr/lib/libcrypto.so.3", b"moved library")],
        &[],
    );

    let pkg = write_config_package(&db, "hello", "2.0-1", &[("usr/share/new.txt", b"new")], &[]);
    let mut ctx = upgrade_context(&root, &db, pkg, "1.0-1", "2.0-1");
    ctx.transaction_paths = Some(Arc::new(HashSet::from([
        "usr/lib/libcrypto.so.3".to_string()
    ])));
    FileExtractionHook.run(&ctx).expect("upgrade");

    assert!(
        root.join("usr/lib/libcrypto.so.3").exists(),
        "file moved to another target must not be removed"
    );
    assert!(root.join("usr/share/new.txt").exists());
}

#[test]
fn remove_keeps_modified_config_as_pacsave() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let db = tmp.path().join("db");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&db).expect("db");

    install_v1(
        &root,
        &db,
        "1.0-1",
        &[("etc/foo.conf", b"v1")],
        &["etc/foo.conf"],
    );
    fs::write(root.join("etc/foo.conf"), "user edited").expect("edit config");

    let ctx = HookContext {
        operation_type: OperationType::Remove,
        pkg_name: "hello".to_string(),
        pkg_version: "1.0-1".to_string(),
        old_version: None,
        pkg_file: None,
        root_dir: root.clone(),
        local_db_dir: db.clone(),
        save_configs: true,
        shell_integration: false,
        scriptlet_mode: ScriptletMode::Auto,
        metadata: None,
        transaction_paths: None,
    };
    FileRemovalHook.run(&ctx).expect("remove");

    assert!(!root.join("etc/foo.conf").exists());
    assert_eq!(read(&root.join("etc/foo.conf.pacsave")), "user edited");
}

#[test]
fn remove_nosave_deletes_modified_config() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let db = tmp.path().join("db");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&db).expect("db");

    install_v1(
        &root,
        &db,
        "1.0-1",
        &[("etc/foo.conf", b"v1")],
        &["etc/foo.conf"],
    );
    fs::write(root.join("etc/foo.conf"), "user edited").expect("edit config");

    let ctx = HookContext {
        operation_type: OperationType::Remove,
        pkg_name: "hello".to_string(),
        pkg_version: "1.0-1".to_string(),
        old_version: None,
        pkg_file: None,
        root_dir: root.clone(),
        local_db_dir: db.clone(),
        save_configs: false,
        shell_integration: false,
        scriptlet_mode: ScriptletMode::Auto,
        metadata: None,
        transaction_paths: None,
    };
    FileRemovalHook.run(&ctx).expect("remove");

    assert!(!root.join("etc/foo.conf").exists());
    assert!(!root.join("etc/foo.conf.pacsave").exists());
}

#[test]
fn remove_unmodified_config_deletes_it() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path().join("root");
    let db = tmp.path().join("db");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&db).expect("db");

    install_v1(
        &root,
        &db,
        "1.0-1",
        &[("etc/foo.conf", b"v1")],
        &["etc/foo.conf"],
    );

    let ctx = HookContext {
        operation_type: OperationType::Remove,
        pkg_name: "hello".to_string(),
        pkg_version: "1.0-1".to_string(),
        old_version: None,
        pkg_file: None,
        root_dir: root.clone(),
        local_db_dir: db.clone(),
        save_configs: true,
        shell_integration: false,
        scriptlet_mode: ScriptletMode::Auto,
        metadata: None,
        transaction_paths: None,
    };
    FileRemovalHook.run(&ctx).expect("remove");

    assert!(!root.join("etc/foo.conf").exists());
    assert!(!root.join("etc/foo.conf.pacsave").exists());
}

#[test]
fn metadata_load_hook_rejects_corrupt_package() {
    let (_root, db_tmp, mut ctx) = test_context(OperationType::Install);
    let bad = db_tmp.path().join("bad.xp");
    fs::write(&bad, b"not a package").expect("write bad");
    ctx.pkg_file = Some(bad);

    let err = MetadataLoadHook.run(&ctx).expect_err("must fail");
    assert!(err.to_string().contains("invalid package metadata"));
}

#[test]
fn hardlink_replaces_an_existing_target() {
    use std::os::unix::fs::MetadataExt;

    let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
    let package = db_tmp.path().join("hardlink-1.0-1-x86_64.xp");
    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);
        let pkginfo =
            b"pkgname = hardlink\npkgver = 1.0-1\npkgdesc = test\narch = x86_64\nsize = 3\n";
        let mut header = tar::Header::new_gnu();
        header.set_path(".PKGINFO").unwrap();
        header.set_size(pkginfo.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &pkginfo[..]).unwrap();

        let content = b"new";
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/tool").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, &content[..]).unwrap();

        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/tool-link").unwrap();
        header.set_entry_type(tar::EntryType::Link);
        header.set_link_name("usr/bin/tool").unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
        builder.finish().unwrap();
    }
    fs::write(&package, zstd::encode_all(&raw_tar[..], 1).unwrap()).unwrap();

    fs::create_dir_all(root.path().join("usr/bin")).unwrap();
    fs::write(root.path().join("usr/bin/tool-link"), b"old").unwrap();
    ctx.pkg_name = "hardlink".to_string();
    ctx.pkg_file = Some(package);

    FileExtractionHook
        .run(&ctx)
        .expect("extract must replace the existing target");

    let target = root.path().join("usr/bin/tool-link");
    assert_eq!(fs::read(&target).unwrap(), b"new");
    let source_meta = fs::metadata(root.path().join("usr/bin/tool")).unwrap();
    let target_meta = fs::metadata(&target).unwrap();
    assert_eq!(
        source_meta.ino(),
        target_meta.ino(),
        "target must be a hardlink"
    );
}
