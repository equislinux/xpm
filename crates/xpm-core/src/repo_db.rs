//! Repository database parsing for ALPM-style `.db` and `.files` archives.
//!
//! This module provides the foundation for sync database support (roadmap
//! phase 5), including parsing package metadata and file listings.

use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use flate2::read::GzDecoder;
use xz2::read::XzDecoder;

use crate::XpmError;
use crate::XpmResult;

/// A package entry stored in a repository database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoEntry {
    pub name: String,
    pub version: String,
    pub filename: Option<String>,
    pub sha256sum: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub arch: Option<String>,
    pub depends: Vec<String>,
    pub opt_depends: Vec<String>,
    pub provides: Vec<String>,
    pub conflicts: Vec<String>,
    pub replaces: Vec<String>,
    pub files: Vec<String>,
}

/// Parsed sync database for a repository (e.g. `core.db`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncDb {
    pub repo: String,
    pub entries: Vec<RepoEntry>,
}

/// In-memory representation of the local package database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalDb {
    pub entries: Vec<RepoEntry>,
}

#[derive(Debug, Default)]
struct ArchivePackage {
    desc: Option<String>,
    depends: Option<String>,
    files: Option<String>,
}

/// Parse a repository sync database archive (`<repo>.db`).
pub fn parse_sync_db(path: &Path, repo_name: &str) -> XpmResult<SyncDb> {
    let bytes = fs::read(path)?;
    parse_sync_db_bytes(&bytes, repo_name)
}

/// Parse a repository files database archive (`<repo>.files`) and merge file
/// listings into an existing [`SyncDb`].
pub fn merge_files_db(path: &Path, db: &mut SyncDb) -> XpmResult<()> {
    let bytes = fs::read(path)?;
    merge_files_db_bytes(&bytes, db)
}

fn parse_sync_db_bytes(bytes: &[u8], repo_name: &str) -> XpmResult<SyncDb> {
    let mut packages = read_repo_archive(bytes)?;
    let mut entries = Vec::new();

    for (pkg_id, pkg) in packages.drain() {
        let mut merged = HashMap::<String, Vec<String>>::new();

        if let Some(desc) = pkg.desc.as_deref() {
            merge_sections(&mut merged, parse_alpm_sections(desc));
        }
        if let Some(depends) = pkg.depends.as_deref() {
            merge_sections(&mut merged, parse_alpm_sections(depends));
        }

        let entry = repo_entry_from_sections(&pkg_id, &merged)?;
        entries.push(entry);
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));

    Ok(SyncDb {
        repo: repo_name.to_string(),
        entries,
    })
}

fn merge_files_db_bytes(bytes: &[u8], db: &mut SyncDb) -> XpmResult<()> {
    let mut packages = read_repo_archive(bytes)?;
    let mut files_by_id = HashMap::<String, Vec<String>>::new();

    for (pkg_id, pkg) in packages.drain() {
        let Some(files_content) = pkg.files else {
            continue;
        };

        let sections = parse_alpm_sections(&files_content);
        let files = sections
            .get("FILES")
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();

        files_by_id.insert(pkg_id, files);
    }

    for entry in &mut db.entries {
        let pkg_id = format!("{}-{}", entry.name, entry.version);
        if let Some(files) = files_by_id.remove(&pkg_id) {
            entry.files = files;
        }
    }

    Ok(())
}

fn read_repo_archive(bytes: &[u8]) -> XpmResult<HashMap<String, ArchivePackage>> {
    let mut archive = open_archive(bytes)?;
    let mut packages = HashMap::<String, ArchivePackage>::new();

    for entry_result in archive.entries()? {
        let mut entry = entry_result.map_err(|e| XpmError::Database(e.to_string()))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }

        let path = entry
            .path()
            .map_err(|e| XpmError::Database(e.to_string()))?
            .into_owned();
        let path_str = path.to_string_lossy();
        let mut parts = path_str.split('/').filter(|p| !p.is_empty());
        let Some(pkg_id) = parts.next() else {
            continue;
        };
        let Some(file_name) = parts.next() else {
            continue;
        };

        let mut content = String::new();
        entry.read_to_string(&mut content)?;

        let pkg = packages.entry(pkg_id.to_string()).or_default();
        match file_name {
            "desc" => pkg.desc = Some(content),
            "depends" => pkg.depends = Some(content),
            "files" => pkg.files = Some(content),
            _ => {}
        }
    }

    Ok(packages)
}

fn open_archive(bytes: &[u8]) -> XpmResult<tar::Archive<Box<dyn Read>>> {
    let reader: Box<dyn Read> = if is_gzip(bytes) {
        Box::new(GzDecoder::new(Cursor::new(bytes.to_vec())))
    } else if is_zstd(bytes) {
        // xpkg repo-add writes zstd-compressed databases by default; ALPM-style
        // `.db` files can also arrive gzip or xz compressed.
        Box::new(
            zstd::Decoder::new(Cursor::new(bytes.to_vec()))
                .map_err(|e| XpmError::Database(format!("failed to decode zstd database: {e}")))?,
        )
    } else if is_xz(bytes) {
        Box::new(XzDecoder::new(Cursor::new(bytes.to_vec())))
    } else {
        Box::new(Cursor::new(bytes.to_vec()))
    };

    Ok(tar::Archive::new(reader))
}

fn is_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

fn is_zstd(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[..4] == [0x28, 0xb5, 0x2f, 0xfd]
}

fn is_xz(bytes: &[u8]) -> bool {
    bytes.len() >= 6 && bytes[..6] == [0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]
}

/// Case-insensitive match of `query` against an entry's name, description or
/// provides (the `xpm search` contract).
pub fn matches_query(entry: &RepoEntry, query: &str) -> bool {
    let query = query.to_lowercase();
    if query.is_empty() {
        return false;
    }

    entry.name.to_lowercase().contains(&query)
        || entry
            .description
            .as_deref()
            .is_some_and(|desc| desc.to_lowercase().contains(&query))
        || entry
            .provides
            .iter()
            .any(|provide| provide.to_lowercase().contains(&query))
}

fn merge_sections(into: &mut HashMap<String, Vec<String>>, from: HashMap<String, Vec<String>>) {
    for (k, mut v) in from {
        into.entry(k).or_default().append(&mut v);
    }
}

fn repo_entry_from_sections(
    pkg_id: &str,
    sections: &HashMap<String, Vec<String>>,
) -> XpmResult<RepoEntry> {
    let name = first_value(sections, "NAME").ok_or_else(|| {
        XpmError::Database(format!("missing %NAME% in repository entry '{pkg_id}'"))
    })?;
    let version = first_value(sections, "VERSION").ok_or_else(|| {
        XpmError::Database(format!("missing %VERSION% in repository entry '{pkg_id}'"))
    })?;

    Ok(RepoEntry {
        name,
        version,
        filename: first_value(sections, "FILENAME"),
        sha256sum: first_value(sections, "SHA256SUM"),
        url: first_value(sections, "URL"),
        description: first_value(sections, "DESC"),
        arch: first_value(sections, "ARCH"),
        depends: values(sections, "DEPENDS"),
        opt_depends: values(sections, "OPTDEPENDS"),
        provides: values(sections, "PROVIDES"),
        conflicts: values(sections, "CONFLICTS"),
        replaces: values(sections, "REPLACES"),
        files: Vec::new(),
    })
}

fn first_value(sections: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    sections.get(key).and_then(|values| values.first()).cloned()
}

fn values(sections: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
    sections.get(key).cloned().unwrap_or_default()
}

fn parse_alpm_sections(input: &str) -> HashMap<String, Vec<String>> {
    let mut map = HashMap::<String, Vec<String>>::new();
    let mut current: Option<String> = None;

    for raw in input.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('%') && line.ends_with('%') && line.len() > 2 {
            let key = line.trim_matches('%').to_ascii_uppercase();
            current = Some(key);
            continue;
        }

        if let Some(section) = &current {
            map.entry(section.clone())
                .or_default()
                .push(line.to_string());
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_zstd_tar(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut raw = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw);
            for (path, contents) in entries {
                let bytes = contents.as_bytes();
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, *path, bytes)
                    .expect("append tar entry");
            }
            builder.finish().expect("finish tar builder");
        }
        zstd::encode_all(&raw[..], 3).expect("compress zstd")
    }

    #[test]
    fn matches_query_searches_name_description_and_provides() {
        let entry = RepoEntry {
            name: "firefox".to_string(),
            version: "128.0-1".to_string(),
            description: Some("Fast, Private Browser".to_string()),
            provides: vec!["browser".to_string()],
            ..Default::default()
        };

        assert!(matches_query(&entry, "fire"));
        assert!(matches_query(&entry, "FIREFOX"), "case-insensitive name");
        assert!(matches_query(&entry, "private"), "description match");
        assert!(matches_query(&entry, "BROWSER"), "provides match");
        assert!(!matches_query(&entry, "chromium"));
        assert!(!matches_query(&entry, ""), "empty query never matches");
    }

    #[test]
    fn parse_sync_db_reads_zstd_xpkg_database() {
        let bytes = make_zstd_tar(&[(
            "hello-1.0-1/desc",
            "%NAME%\nhello\n\n%VERSION%\n1.0-1\n\n%DESC%\nzstd database entry\n\n%ARCH%\nx86_64\n\n%FILENAME%\nhello-1.0-1-x86_64.xp\n",
        )]);

        let db = parse_sync_db_bytes(&bytes, "x").expect("parse zstd sync db");

        assert_eq!(db.entries.len(), 1);
        assert_eq!(db.entries[0].name, "hello");
        assert_eq!(db.entries[0].version, "1.0-1");
        assert_eq!(
            db.entries[0].filename.as_deref(),
            Some("hello-1.0-1-x86_64.xp")
        );
    }

    fn make_gzip_tar(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut gz);
            for (path, contents) in entries {
                let bytes = contents.as_bytes();
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder
                    .append_data(&mut header, *path, bytes)
                    .expect("append tar entry");
            }
            builder.finish().expect("finish tar builder");
        }
        gz.finish().expect("finish gzip stream")
    }

    #[test]
    fn parse_sync_db_reads_desc_and_depends() {
        let bytes = make_gzip_tar(&[
            (
                "hello-1.0-1/desc",
                "%NAME%\nhello\n\n%VERSION%\n1.0-1\n\n%FILENAME%\nhello-1.0-1-x86_64.xp\n\n%SHA256SUM%\nabc123\n\n%URL%\nhttps://github.com/xlnux/hello\n\n%DESC%\nhello package\n\n%ARCH%\nx86_64\n",
            ),
            (
                "hello-1.0-1/depends",
                "%DEPENDS%\nlibc>=2.39\n\n%PROVIDES%\nhello-bin\n\n%CONFLICTS%\nhello-git\n",
            ),
            (
                "world-2.0-1/desc",
                "%NAME%\nworld\n\n%VERSION%\n2.0-1\n\n%DESC%\nworld package\n",
            ),
        ]);

        let db = parse_sync_db_bytes(&bytes, "core").expect("parse sync db");

        assert_eq!(db.repo, "core");
        assert_eq!(db.entries.len(), 2);

        let hello = db
            .entries
            .iter()
            .find(|e| e.name == "hello")
            .expect("hello entry exists");
        assert_eq!(hello.version, "1.0-1");
        assert_eq!(hello.description.as_deref(), Some("hello package"));
        assert_eq!(hello.arch.as_deref(), Some("x86_64"));
        assert_eq!(hello.filename.as_deref(), Some("hello-1.0-1-x86_64.xp"));
        assert_eq!(hello.sha256sum.as_deref(), Some("abc123"));
        assert_eq!(hello.url.as_deref(), Some("https://github.com/xlnux/hello"));
        assert_eq!(hello.depends, vec!["libc>=2.39"]);
        assert_eq!(hello.provides, vec!["hello-bin"]);
        assert_eq!(hello.conflicts, vec!["hello-git"]);
    }

    #[test]
    fn merge_files_db_adds_file_lists() {
        let db_bytes = make_gzip_tar(&[(
            "hello-1.0-1/desc",
            "%NAME%\nhello\n\n%VERSION%\n1.0-1\n\n%DESC%\nhello package\n",
        )]);
        let files_bytes = make_gzip_tar(&[(
            "hello-1.0-1/files",
            "%FILES%\nusr/\nusr/bin/\nusr/bin/hello\n",
        )]);

        let mut db = parse_sync_db_bytes(&db_bytes, "core").expect("parse sync db");
        merge_files_db_bytes(&files_bytes, &mut db).expect("merge files db");

        assert_eq!(db.entries.len(), 1);
        assert_eq!(
            db.entries[0].files,
            vec!["usr/", "usr/bin/", "usr/bin/hello"]
        );
    }

    #[test]
    fn parse_sync_db_reports_missing_required_fields() {
        let bytes = make_gzip_tar(&[("broken/desc", "%NAME%\nbroken\n")]);
        let err = parse_sync_db_bytes(&bytes, "core").expect_err("must fail");
        assert!(format!("{err}").contains("missing %VERSION%"));
    }

    #[test]
    fn parse_plain_tar_is_supported() {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            let contents = "%NAME%\nplain\n\n%VERSION%\n1.0-1\n";
            let bytes = contents.as_bytes();
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "plain-1.0-1/desc", bytes)
                .expect("append tar entry");
            builder.finish().expect("finish tar builder");
        }

        let db = parse_sync_db_bytes(&tar_bytes, "core").expect("parse plain tar");
        assert_eq!(db.entries.len(), 1);
        assert_eq!(db.entries[0].name, "plain");
    }

    // ===== Repository Database Test Suite (#56) =====
    // Tests for real Arch/x-linux .db file compatibility

    #[test]
    fn parse_arch_style_db_without_xpkg_fields() {
        // Standard Arch .db format (no FILENAME, SHA256SUM, URL).
        // This validates backward compatibility when parsing real Arch databases.
        let bytes = make_gzip_tar(&[
            (
                "linux-6.1.0-1/desc",
                "%NAME%\nlinux\n\n%VERSION%\n6.1.0-1\n\n%DESC%\nThe Linux kernel and modules\n\n%ARCH%\nx86_64\n",
            ),
            (
                "linux-6.1.0-1/depends",
                "%DEPENDS%\ncoreutils\n\n%OPTDEPENDS%\ncrda\n\n%PROVIDES%\nlinux-modules\n\n%CONFLICTS%\nlinux-lts\n",
            ),
        ]);

        let db = parse_sync_db_bytes(&bytes, "core").expect("parse arch-style db");
        assert_eq!(db.entries.len(), 1);

        let linux = &db.entries[0];
        assert_eq!(linux.name, "linux");
        assert_eq!(linux.version, "6.1.0-1");
        assert_eq!(
            linux.description.as_deref(),
            Some("The Linux kernel and modules")
        );
        assert_eq!(linux.arch.as_deref(), Some("x86_64"));
        // These fields should be None for standard Arch .db
        assert!(linux.filename.is_none());
        assert!(linux.sha256sum.is_none());
        assert!(linux.url.is_none());
        // Standard fields should still parse
        assert_eq!(linux.depends, vec!["coreutils"]);
        assert_eq!(linux.opt_depends, vec!["crda"]);
        assert_eq!(linux.provides, vec!["linux-modules"]);
        assert_eq!(linux.conflicts, vec!["linux-lts"]);
    }

    #[test]
    fn parse_xpkg_extended_db_with_metadata_fields() {
        // x-linux extended .db format with FILENAME, SHA256SUM, URL.
        // This validates x-repo's metadata-rich repository setup.
        let bytes = make_gzip_tar(&[
            (
                "xpkg-0.1.0-1/desc",
                "%NAME%\nxpkg\n\n%VERSION%\n0.1.0-1\n\n%FILENAME%\nxpkg-0.1.0-1-x86_64.xp\n\n%SHA256SUM%\ndeadbeefcafebabe0000000000000000deadbeefcafebabe0000000000000000\n\n%URL%\nhttps://github.com/xlnux/xpkg\n\n%DESC%\nX distribution package builder\n\n%ARCH%\nx86_64\n",
            ),
            (
                "xpkg-0.1.0-1/depends",
                "%DEPENDS%\nrust\n\n%PROVIDES%\nxpkg-core\n",
            ),
        ]);

        let db = parse_sync_db_bytes(&bytes, "x").expect("parse xpkg-extended db");
        assert_eq!(db.entries.len(), 1);

        let xpkg = &db.entries[0];
        assert_eq!(xpkg.name, "xpkg");
        assert_eq!(xpkg.version, "0.1.0-1");
        // Extended fields should be present
        assert_eq!(xpkg.filename.as_deref(), Some("xpkg-0.1.0-1-x86_64.xp"));
        assert_eq!(
            xpkg.sha256sum.as_deref(),
            Some("deadbeefcafebabe0000000000000000deadbeefcafebabe0000000000000000")
        );
        assert_eq!(xpkg.url.as_deref(), Some("https://github.com/xlnux/xpkg"));
        assert_eq!(xpkg.depends, vec!["rust"]);
        assert_eq!(xpkg.provides, vec!["xpkg-core"]);
    }

    #[test]
    fn parse_complex_dependency_structure() {
        // Test with complex, multi-dependency package (realistic scenario).
        let bytes = make_gzip_tar(&[
            (
                "gcc-13.1.0-1/desc",
                "%NAME%\ngcc\n\n%VERSION%\n13.1.0-1\n\n%DESC%\nGNU compiler collection\n\n%ARCH%\nx86_64\n",
            ),
            (
                "gcc-13.1.0-1/depends",
                "%DEPENDS%\nzstd\n%DEPENDS%\ngmp\n%DEPENDS%\nmpfr\n%DEPENDS%\nmpc\n%DEPENDS%\nisl\n%OPTDEPENDS%\nlib32-glibc\n%PROVIDES%\ngcc\n%PROVIDES%\ncc\n%PROVIDES%\nc++\n%CONFLICTS%\ngcc-libs\n",
            ),
        ]);

        let db = parse_sync_db_bytes(&bytes, "core").expect("parse complex db");
        let gcc = &db.entries[0];

        assert_eq!(gcc.depends.len(), 5);
        assert!(gcc.depends.contains(&"zstd".to_string()));
        assert!(gcc.depends.contains(&"gmp".to_string()));
        assert!(gcc.depends.contains(&"mpfr".to_string()));

        assert_eq!(gcc.opt_depends, vec!["lib32-glibc"]);
        assert_eq!(gcc.provides.len(), 3);
        assert!(gcc.provides.contains(&"gcc".to_string()));
        assert!(gcc.provides.contains(&"cc".to_string()));
    }

    #[test]
    fn parse_minimal_valid_db() {
        // Minimum required fields: NAME, VERSION.
        // This validates parser robustness for minimal entries.
        let bytes = make_gzip_tar(&[("tiny-1.0-1/desc", "%NAME%\ntiny\n\n%VERSION%\n1.0-1\n")]);

        let db = parse_sync_db_bytes(&bytes, "test").expect("parse minimal db");
        assert_eq!(db.entries.len(), 1);
        assert_eq!(db.entries[0].name, "tiny");
        assert_eq!(db.entries[0].version, "1.0-1");
        // All optional fields should be empty/None
        assert!(db.entries[0].description.is_none());
        assert!(db.entries[0].arch.is_none());
        assert!(db.entries[0].depends.is_empty());
        assert!(db.entries[0].provides.is_empty());
    }

    #[test]
    fn parse_large_repository_db() {
        // Simulate a repo with many packages (stress test for parser).
        // Use zero-padded package IDs to maintain alphabetical tar order.
        let mut entries = Vec::new();
        for i in 1..=50 {
            let padded = format!("{:03}", i); // package001, package002, etc
            let name = format!("pkg{}", padded);
            let version = format!("{}.0.0-1", padded);
            let desc = format!(
                "%NAME%\n{}\n\n%VERSION%\n{}\n\n%DESC%\nTest package {}\n",
                name, version, i
            );
            entries.push((format!("{}/desc", version), desc));
        }

        let entry_refs: Vec<_> = entries
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        let bytes = make_gzip_tar(&entry_refs);

        let db = parse_sync_db_bytes(&bytes, "large").expect("parse large repo");
        assert_eq!(db.entries.len(), 50, "Should parse all 50 packages");
        // Verify first and last entries are present with unique names
        let names: std::collections::HashSet<_> = db.entries.iter().map(|e| &e.name).collect();
        assert_eq!(names.len(), 50, "All package names should be unique");
    }

    #[test]
    fn parse_db_with_optional_arch_field() {
        // Some packages may omit ARCH; should default or be None.
        let bytes = make_gzip_tar(&[(
            "noarch-1.0-1/desc",
            "%NAME%\nnoarch\n\n%VERSION%\n1.0-1\n\n%DESC%\nNo architecture specified\n",
        )]);

        let db = parse_sync_db_bytes(&bytes, "extra").expect("parse db without arch");
        assert_eq!(db.entries[0].arch, None); // Our parser doesn't force a default
    }

    #[test]
    fn merge_files_with_multiple_packages() {
        // Test merging .files for multiple packages simultaneously.
        let db_bytes = make_gzip_tar(&[
            (
                "bin-1.0-1/desc",
                "%NAME%\nbin\n\n%VERSION%\n1.0-1\n\n%DESC%\nbinaries\n",
            ),
            (
                "lib-1.0-1/desc",
                "%NAME%\nlib\n\n%VERSION%\n1.0-1\n\n%DESC%\nlibraries\n",
            ),
        ]);

        let files_bytes = make_gzip_tar(&[
            ("bin-1.0-1/files", "%FILES%\nusr/bin/\nusr/bin/tool\n"),
            (
                "lib-1.0-1/files",
                "%FILES%\nusr/lib/\nusr/lib64/\nusr/lib/libfoo.so\n",
            ),
        ]);

        let mut db = parse_sync_db_bytes(&db_bytes, "extra").expect("parse multi-pkg db");
        merge_files_db_bytes(&files_bytes, &mut db).expect("merge files");

        assert_eq!(db.entries.len(), 2);
        assert_eq!(db.entries[0].files, vec!["usr/bin/", "usr/bin/tool"]);
        assert_eq!(
            db.entries[1].files,
            vec!["usr/lib/", "usr/lib64/", "usr/lib/libfoo.so"]
        );
    }

    #[test]
    fn real_db_file_from_disk() {
        // Test parsing a real .db file if it exists in the workspace.
        let real_db_path = Path::new(
            "/home/xscriptor/Documents/repos/xpkgrepos/x-repo/public/repo/x86_64/x.db.tar.gz",
        );

        if real_db_path.exists() {
            let bytes = fs::read(real_db_path).expect("read real x.db");
            let db = parse_sync_db_bytes(&bytes, "x").expect("parse real x.db");

            // Validate structure
            assert!(
                !db.entries.is_empty(),
                "x.db should contain at least one entry"
            );
            for entry in &db.entries {
                // Every entry must have name and version
                assert!(!entry.name.is_empty(), "entry name must not be empty");
                assert!(!entry.version.is_empty(), "entry version must not be empty");
            }

            // Check for expected xfetch entry if present
            if let Some(xfetch) = db.entries.iter().find(|e| e.name == "xfetch") {
                assert_eq!(xfetch.version, "0.1.0-1");
                // Should have extended metadata fields
                assert!(xfetch.filename.is_some(), "xfetch should have FILENAME");
                assert!(xfetch.sha256sum.is_some(), "xfetch should have SHA256SUM");
            }
        }
    }

    #[test]
    fn parse_db_with_version_variants() {
        // Test parsing versions in different formats (semantic, pre-release, etc).
        let bytes = make_gzip_tar(&[
            ("pkg1-1.0.0-1/desc", "%NAME%\npkg1\n\n%VERSION%\n1.0.0-1\n"),
            (
                "pkg2-2.5beta-2/desc",
                "%NAME%\npkg2\n\n%VERSION%\n2.5beta-2\n",
            ),
            (
                "pkg3-0.0.1rc1-1/desc",
                "%NAME%\npkg3\n\n%VERSION%\n0.0.1rc1-1\n",
            ),
        ]);

        let db = parse_sync_db_bytes(&bytes, "test").expect("parse versions");
        assert_eq!(db.entries[0].version, "1.0.0-1");
        assert_eq!(db.entries[1].version, "2.5beta-2");
        assert_eq!(db.entries[2].version, "0.0.1rc1-1");
    }

    #[test]
    fn parse_db_ignores_missing_optional_depends_section() {
        // Some entries may not have depends/ file at all.
        let bytes = make_gzip_tar(&[(
            "standalone-1.0-1/desc",
            "%NAME%\nstandalone\n\n%VERSION%\n1.0-1\n",
        )]);

        let db = parse_sync_db_bytes(&bytes, "test").expect("parse without depends");
        assert_eq!(db.entries.len(), 1);
        assert!(db.entries[0].depends.is_empty());
        assert!(db.entries[0].provides.is_empty());
    }
}
