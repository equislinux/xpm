//! Integration tests for repository database parsing (#56) through the public,
//! path-based API (`parse_sync_db` / `merge_files_db`).
//!
//! The fixtures are built in a temporary directory: an xpkg-extended database
//! (FILENAME/SHA256SUM/URL), a plain Arch-style database, and a matching
//! `.files` archive. A real `[x]` repository can be exercised by pointing
//! `XPM_TEST_REPO_DIR` at a directory containing `x.db.tar.gz` and `x.files`
//! (the test is skipped when the variable is unset).

use std::fs;
use std::io::Write;
use std::path::Path;

use xpm_core::repo_db::{merge_files_db, parse_sync_db};

/// Writes a gzip-compressed tar archive with the given entries.
fn write_gzip_tar(path: &Path, entries: &[(&str, &str)]) {
    let file = fs::File::create(path).expect("create archive");
    let gz = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);

    for (entry_path, contents) in entries {
        let bytes = contents.as_bytes();
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, *entry_path, bytes)
            .expect("append tar entry");
    }

    let gz = builder.into_inner().expect("finish tar builder");
    let mut file = gz.finish().expect("finish gzip stream");
    file.flush().expect("flush archive");
}

#[test]
fn parses_xpkg_extended_database_from_path() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db_path = tmp.path().join("x.db.tar.gz");
    write_gzip_tar(
        &db_path,
        &[
            (
                "hello-1.0-1/desc",
                "%NAME%\nhello\n\n%VERSION%\n1.0-1\n\n%FILENAME%\nhello-1.0-1-x86_64.xp\n\n%SHA256SUM%\nabc123\n\n%URL%\nhttps://github.com/equislinux/hello\n\n%DESC%\nhello package\n\n%ARCH%\nx86_64\n",
            ),
            (
                "hello-1.0-1/depends",
                "%DEPENDS%\nlibc>=2.39\n\n%PROVIDES%\nhello-bin\n",
            ),
        ],
    );

    let db = parse_sync_db(&db_path, "x").expect("parse xpkg-extended db");
    assert_eq!(db.repo, "x");
    assert_eq!(db.entries.len(), 1);

    let hello = &db.entries[0];
    assert_eq!(hello.name, "hello");
    assert_eq!(hello.version, "1.0-1");
    assert_eq!(hello.filename.as_deref(), Some("hello-1.0-1-x86_64.xp"));
    assert_eq!(hello.sha256sum.as_deref(), Some("abc123"));
    assert_eq!(
        hello.url.as_deref(),
        Some("https://github.com/equislinux/hello")
    );
    assert_eq!(hello.depends, vec!["libc>=2.39"]);
    assert_eq!(hello.provides, vec!["hello-bin"]);
}

#[test]
fn parses_arch_style_database_without_extended_fields() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db_path = tmp.path().join("core.db.tar.gz");
    write_gzip_tar(
        &db_path,
        &[
            (
                "bash-5.3.0-1/desc",
                "%NAME%\nbash\n\n%VERSION%\n5.3.0-1\n\n%DESC%\nThe GNU Bourne Again shell\n\n%ARCH%\nx86_64\n",
            ),
            (
                "bash-5.3.0-1/depends",
                "%DEPENDS%\nglibc\nreadline\n",
            ),
        ],
    );

    let db = parse_sync_db(&db_path, "core").expect("parse arch-style db");
    assert_eq!(db.entries.len(), 1);

    let bash = &db.entries[0];
    assert_eq!(bash.name, "bash");
    assert_eq!(bash.version, "5.3.0-1");
    // Arch databases carry no fetch metadata.
    assert!(bash.filename.is_none());
    assert!(bash.sha256sum.is_none());
    assert!(bash.url.is_none());
    assert_eq!(bash.depends, vec!["glibc", "readline"]);
}

#[test]
fn merges_files_database_from_path() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db_path = tmp.path().join("core.db.tar.gz");
    let files_path = tmp.path().join("core.files");

    write_gzip_tar(
        &db_path,
        &[(
            "hello-1.0-1/desc",
            "%NAME%\nhello\n\n%VERSION%\n1.0-1\n\n%DESC%\nhello package\n",
        )],
    );
    write_gzip_tar(
        &files_path,
        &[(
            "hello-1.0-1/files",
            "%FILES%\nusr/\nusr/bin/\nusr/bin/hello\n",
        )],
    );

    let mut db = parse_sync_db(&db_path, "core").expect("parse db");
    assert!(db.entries[0].files.is_empty());

    merge_files_db(&files_path, &mut db).expect("merge files db");
    assert_eq!(
        db.entries[0].files,
        vec!["usr/", "usr/bin/", "usr/bin/hello"]
    );
}

#[test]
fn merges_second_package_files_by_name_version() {
    let tmp = tempfile::TempDir::new().expect("tmp");
    let db_path = tmp.path().join("extra.db.tar.gz");
    let files_path = tmp.path().join("extra.files");

    write_gzip_tar(
        &db_path,
        &[
            (
                "a-1.0-1/desc",
                "%NAME%\na\n\n%VERSION%\n1.0-1\n\n%DESC%\na package\n",
            ),
            (
                "b-2.0-1/desc",
                "%NAME%\nb\n\n%VERSION%\n2.0-1\n\n%DESC%\nb package\n",
            ),
        ],
    );
    write_gzip_tar(
        &files_path,
        &[
            ("a-1.0-1/files", "%FILES%\nusr/bin/a\n"),
            ("b-2.0-1/files", "%FILES%\nusr/bin/b\n"),
        ],
    );

    let mut db = parse_sync_db(&db_path, "extra").expect("parse db");
    merge_files_db(&files_path, &mut db).expect("merge files");

    let a = db.entries.iter().find(|e| e.name == "a").expect("a");
    let b = db.entries.iter().find(|e| e.name == "b").expect("b");
    assert_eq!(a.files, vec!["usr/bin/a"]);
    assert_eq!(b.files, vec!["usr/bin/b"]);
}

/// Optional check against a real repository tree, driven by an env var so the
/// suite stays hermetic (no hardcoded machine paths).
///
/// `XPM_TEST_REPO_DIR=/path/to/repo/x86_64` must contain `x.db.tar.gz`.
#[test]
fn parses_real_repository_when_available() {
    let Ok(repo_dir) = std::env::var("XPM_TEST_REPO_DIR") else {
        eprintln!("skipping: XPM_TEST_REPO_DIR is not set");
        return;
    };

    let db_path = Path::new(&repo_dir).join("x.db.tar.gz");
    if !db_path.exists() {
        eprintln!("skipping: {} does not exist", db_path.display());
        return;
    }

    let mut db = parse_sync_db(&db_path, "x").expect("parse real x.db");
    assert!(!db.entries.is_empty(), "real x.db must contain packages");
    for entry in &db.entries {
        assert!(!entry.name.is_empty());
        assert!(!entry.version.is_empty());
    }

    let files_path = Path::new(&repo_dir).join("x.files");
    if files_path.exists() {
        merge_files_db(&files_path, &mut db).expect("merge real x.files");
        let with_files = db.entries.iter().filter(|e| !e.files.is_empty()).count();
        eprintln!(
            "real repo: {} packages, {} with file lists",
            db.entries.len(),
            with_files
        );
    }
}
