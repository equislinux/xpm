use super::*;
use tempfile::TempDir;

fn entry(path: &str, file_type: MtreeFileType) -> MtreeEntry {
    MtreeEntry {
        path: path.into(),
        file_type,
        mode: 0o644,
        uid: 0,
        gid: 0,
        size: 0,
        sha256: None,
        link_target: None,
    }
}

fn sample_entries() -> Vec<MtreeEntry> {
    vec![
        entry("./usr", MtreeFileType::Dir),
        entry("./usr/bin", MtreeFileType::Dir),
        entry("./usr/bin/hello", MtreeFileType::File),
        entry("./usr/lib/libfoo.so", MtreeFileType::Link),
        entry("./usr/lib/libfoo.so.1", MtreeFileType::File),
    ]
}

#[test]
fn mtree_paths_include_dirs_files_and_symlinks() {
    let paths = mtree_paths(&sample_entries());
    assert_eq!(
        paths,
        vec![
            "usr/",
            "usr/bin/",
            "usr/bin/hello",
            "usr/lib/libfoo.so",
            "usr/lib/libfoo.so.1",
        ]
    );
}

#[test]
fn mtree_paths_normalize_prefixes_and_skip_root() {
    let entries = vec![
        entry("./", MtreeFileType::Dir),
        entry(".", MtreeFileType::Dir),
        entry("/etc/xpm.conf", MtreeFileType::File),
        entry("usr//bin/./tool", MtreeFileType::File),
        entry("../escape", MtreeFileType::File),
        entry("usr/bin/tool", MtreeFileType::File),
    ];

    let paths = mtree_paths(&entries);
    assert_eq!(paths, vec!["etc/xpm.conf", "usr/bin/tool"]);
}

#[test]
fn write_files_emits_pacman_header() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    write_files(&local_db, "test", &sample_entries()).expect("write files");

    let raw = fs::read_to_string(local_db.join("test").join(FILES_FILE)).expect("read");
    assert_eq!(
        raw,
        "%FILES%\nusr/\nusr/bin/\nusr/bin/hello\nusr/lib/libfoo.so\nusr/lib/libfoo.so.1\n"
    );
}

#[test]
fn backup_list_roundtrip_and_legacy_empty() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    assert!(read_backup_list(&local_db, "legacy")
        .expect("legacy reads")
        .is_empty());

    let paths = vec!["etc/foo.conf".to_string(), "etc/bar.conf".to_string()];
    write_backup_list(&local_db, "hello", &paths).expect("write backup");
    assert_eq!(read_backup_list(&local_db, "hello").expect("read"), paths);
}

#[test]
fn mtree_roundtrip_and_legacy_none() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    assert_eq!(read_mtree(&local_db, "legacy").expect("read"), None);

    let raw = b"#mtree\n./etc type=dir mode=0755 uid=0 gid=0\n";
    write_mtree(&local_db, "hello", raw).expect("write mtree");
    assert_eq!(
        read_mtree(&local_db, "hello").expect("read").as_deref(),
        Some(&raw[..])
    );
}

#[test]
fn metadata_entries_are_never_part_of_the_manifest() {
    // Real makepkg .MTREE files list .PKGINFO/.BUILDINFO/.MTREE too.
    let entries = vec![
        entry("./.PKGINFO", MtreeFileType::File),
        entry("./.BUILDINFO", MtreeFileType::File),
        entry("./.MTREE", MtreeFileType::File),
        entry("./usr/bin/tool", MtreeFileType::File),
    ];
    let paths = mtree_paths(&entries);
    assert_eq!(paths, vec!["usr/bin/tool"]);

    let mut tool = entry("./usr/bin/tool", MtreeFileType::File);
    tool.sha256 = Some("abc".into());
    let mut pkginfo = entry("./.PKGINFO", MtreeFileType::File);
    pkginfo.sha256 = Some("def".into());
    let map = mtree_hash_map(&[pkginfo, tool]);
    assert_eq!(map.len(), 1);
    assert!(map.contains_key("usr/bin/tool"));
}

#[test]
fn hash_map_uses_normalized_paths_and_skips_dirs_and_links() {
    let mut file = entry("./etc/foo.conf", MtreeFileType::File);
    file.sha256 = Some("aaa".into());
    let mut dir = entry("./etc", MtreeFileType::Dir);
    dir.sha256 = Some("ignored".into());
    let link = entry("./etc/link", MtreeFileType::Link);

    let map = mtree_hash_map(&[file, dir, link]);
    assert_eq!(map.get("etc/foo.conf").map(String::as_str), Some("aaa"));
    assert_eq!(map.len(), 1);
}

#[test]
fn read_files_skips_header_and_blank_lines() {
    let tmp = TempDir::new().expect("tmp");
    let pkg_dir = tmp.path().join("local").join("test");
    fs::create_dir_all(&pkg_dir).expect("create pkg dir");
    fs::write(
        pkg_dir.join(FILES_FILE),
        "%FILES%\n\nusr/\nusr/bin/hello\n%sOMETHING%\n",
    )
    .expect("write manifest");

    let files = read_files(tmp.path().join("local").as_path(), "test").expect("read files");
    assert_eq!(files, vec!["usr/", "usr/bin/hello"]);
}

#[test]
fn read_files_roundtrips_written_manifest() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    write_files(&local_db, "test", &sample_entries()).expect("write files");
    let files = read_files(&local_db, "test").expect("read files");
    assert_eq!(files, mtree_paths(&sample_entries()));
}

#[test]
fn read_files_missing_manifest_is_empty() {
    let tmp = TempDir::new().expect("tmp");

    let files = read_files(tmp.path(), "legacy").expect("legacy does not fail");
    assert!(files.is_empty());
}

#[test]
fn origin_roundtrip_and_missing_are_none() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    assert_eq!(read_origin(&local_db, "legacy"), None);

    write_origin(&local_db, "kitty", "x").expect("write origin");
    assert_eq!(read_origin(&local_db, "kitty").as_deref(), Some("x"));

    write_origin(&local_db, "empty", "").expect("write empty origin");
    assert_eq!(read_origin(&local_db, "empty"), None);
}

#[test]
fn changelog_reads_or_is_none() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    assert!(read_changelog(&local_db, "legacy").expect("read").is_none());

    let pkg_dir = local_db.join("hello");
    fs::create_dir_all(&pkg_dir).expect("create pkg dir");
    fs::write(pkg_dir.join(CHANGELOG_FILE), "1.0-1: initial\n").expect("changelog");

    assert_eq!(
        read_changelog(&local_db, "hello").expect("read").as_deref(),
        Some("1.0-1: initial\n")
    );
}

#[test]
fn depends_roundtrip_and_legacy_is_none() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    assert_eq!(read_depends(&local_db, "legacy").expect("read"), None);

    let deps = vec!["glibc".to_string(), "libx11>=1.8".to_string()];
    write_depends(&local_db, "hello", &deps).expect("write");
    assert_eq!(read_depends(&local_db, "hello").expect("read"), Some(deps));

    write_depends(&local_db, "empty", &[]).expect("write empty");
    assert_eq!(
        read_depends(&local_db, "empty").expect("read"),
        Some(Vec::new()),
        "an empty record is not the same as a missing one"
    );
}

#[test]
fn provides_roundtrip_and_missing_is_empty() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    assert!(read_provides(&local_db, "legacy").expect("read").is_empty());

    write_provides(&local_db, "hello", &["hello-bin".to_string()]).expect("write");
    assert_eq!(
        read_provides(&local_db, "hello").expect("read"),
        vec!["hello-bin".to_string()]
    );
}

#[test]
fn version_roundtrip_and_missing_are_none() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");

    assert_eq!(read_version(&local_db, "legacy"), None);

    let pkg_dir = local_db.join("kitty");
    fs::create_dir_all(&pkg_dir).expect("create pkg dir");
    fs::write(pkg_dir.join(VERSION_FILE), "0.44.0-1\n").expect("write version");

    assert_eq!(
        read_version(&local_db, "kitty").as_deref(),
        Some("0.44.0-1")
    );
}
