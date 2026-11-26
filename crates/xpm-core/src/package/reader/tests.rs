use super::*;
use std::io::Write;

/// Build a minimal .xp archive (tar.zst) in memory and write to a temp file.
fn make_test_package(dir: &Path) -> PathBuf {
    let pkg_path = dir.join("test-1.0.0-1-x86_64.xp");

    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);

        // .PKGINFO
        let pkginfo = b"pkgname = test\npkgver = 1.0.0-1\n\
                pkgdesc = A test package\narch = x86_64\nsize = 42\n";
        let mut header = tar::Header::new_gnu();
        header.set_path(".PKGINFO").unwrap();
        header.set_size(pkginfo.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &pkginfo[..]).unwrap();

        // .BUILDINFO
        let buildinfo = b"pkgname = test\npkgver = 1.0.0-1\n\
                buildtool = xpkg 0.1.0\ncompress = zstd\n";
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
                ./usr/bin/test type=file mode=0755 size=5 uid=0 gid=0\n";
        let mut header = tar::Header::new_gnu();
        header.set_path(".MTREE").unwrap();
        header.set_size(mtree.len() as u64);
        header.set_mode(0o644);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &mtree[..]).unwrap();

        // usr/ directory
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/").unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();

        // usr/bin/ directory
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/").unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();

        // usr/bin/test file
        let content = b"hello";
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/test").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &content[..]).unwrap();

        builder.finish().unwrap();
    }

    // Compress with zstd.
    let compressed = zstd::encode_all(&raw_tar[..], 3).unwrap();
    let mut file = File::create(&pkg_path).unwrap();
    file.write_all(&compressed).unwrap();

    pkg_path
}

#[test]
fn read_metadata_from_xp() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = make_test_package(dir.path());

    let md = read_metadata(&pkg).unwrap();
    assert_eq!(md.meta.name, "test");
    assert_eq!(md.meta.version, "1.0.0");
    assert_eq!(md.meta.release, "1");
    assert_eq!(md.meta.description, "A test package");
    assert!(md.buildinfo.is_some());
    assert_eq!(md.buildinfo.unwrap().buildtool, "xpkg 0.1.0");
    assert_eq!(md.mtree.len(), 3);
}

#[test]
fn list_files_excludes_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = make_test_package(dir.path());

    let files = list_files(&pkg).unwrap();
    assert_eq!(files.len(), 3); // usr/, usr/bin/, usr/bin/test
    assert!(files
        .iter()
        .all(|f| !f.path.to_string_lossy().starts_with('.')));
}

#[test]
fn extract_to_directory() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = make_test_package(dir.path());
    let dest = dir.path().join("extracted");
    std::fs::create_dir_all(&dest).unwrap();

    let extracted = extract_to(&pkg, &dest).unwrap();
    assert!(!extracted.is_empty());
    assert!(dest.join("usr/bin/test").exists());

    let content = std::fs::read_to_string(dest.join("usr/bin/test")).unwrap();
    assert_eq!(content, "hello");
}

#[test]
fn read_raw_entry_pkginfo() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = make_test_package(dir.path());

    let data = read_raw_entry(&pkg, ".PKGINFO").unwrap();
    assert!(data.is_some());
    let text = String::from_utf8(data.unwrap()).unwrap();
    assert!(text.contains("pkgname = test"));
}

#[test]
fn read_raw_entry_missing() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = make_test_package(dir.path());

    let data = read_raw_entry(&pkg, ".INSTALL").unwrap();
    assert!(data.is_none());
}

#[test]
fn detect_compression_zstd() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = make_test_package(dir.path());
    assert_eq!(detect_compression(&pkg).unwrap(), Compression::Zstd);
}

#[test]
fn missing_pkginfo_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let pkg_path = dir.path().join("bad.xp");

    // Create archive without .PKGINFO
    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);
        let content = b"hello";
        let mut header = tar::Header::new_gnu();
        header.set_path("usr/bin/test").unwrap();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &content[..]).unwrap();
        builder.finish().unwrap();
    }
    let compressed = zstd::encode_all(&raw_tar[..], 3).unwrap();
    std::fs::write(&pkg_path, compressed).unwrap();

    let result = read_metadata(&pkg_path);
    assert!(result.is_err());
}

#[test]
fn nonexistent_file_returns_error() {
    let result = read_metadata(Path::new("/nonexistent/package.xp"));
    assert!(result.is_err());
}

#[test]
fn read_metadata_caches_files_scriptlet_mtree_and_backup_hash() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("meta-1.0-1-x86_64.xp");
    let mut raw_tar = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut raw_tar);
        let append =
            |builder: &mut tar::Builder<&mut Vec<u8>>, path: &str, content: &[u8], mode: u32| {
                let mut header = tar::Header::new_gnu();
                header.set_path(path).unwrap();
                header.set_size(content.len() as u64);
                header.set_mode(mode);
                header.set_uid(0);
                header.set_gid(0);
                header.set_cksum();
                builder.append(&header, content).unwrap();
            };

        let pkginfo = b"pkgname = meta\npkgver = 1.0-1\npkgdesc = meta\narch = x86_64\nsize = 6\nbackup = etc/app.conf\n";
        append(&mut builder, ".PKGINFO", pkginfo, 0o644);
        append(&mut builder, ".INSTALL", b"pre_install() { :; }\n", 0o644);
        let mtree = b"#mtree\n./etc/app.conf type=file mode=0644 size=6 uid=0 gid=0\n";
        append(&mut builder, ".MTREE", mtree, 0o644);
        let mut header = tar::Header::new_gnu();
        header.set_path("etc/").unwrap();
        header.set_size(0);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_uid(0);
        header.set_gid(0);
        header.set_cksum();
        builder.append(&header, &[][..]).unwrap();
        append(&mut builder, "etc/app.conf", b"hello\n", 0o644);
        append(&mut builder, "usr/bin/tool", b"bin\n", 0o755);
        builder.finish().unwrap();
    }
    std::fs::write(&pkg, zstd::encode_all(&raw_tar[..], 3).unwrap()).unwrap();

    let metadata = read_metadata(&pkg).unwrap();
    assert!(metadata.files.contains(&"etc/app.conf".to_string()));
    assert!(metadata.files.contains(&"usr/bin/tool".to_string()));
    assert!(metadata.files.contains(&"etc/".to_string()));
    assert!(metadata.install_script.is_some());
    assert!(metadata.mtree_raw.is_some());
    assert_eq!(metadata.mtree.len(), 1);
    assert_eq!(
        metadata.backup_md5.get("etc/app.conf"),
        Some(&format!("{:x}", Md5::digest(b"hello\n")))
    );
}
