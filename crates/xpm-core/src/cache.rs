//! Local package-cache lookup.
//!
//! `xpm rollback` needs the *old* package file to undo an upgrade or a
//! removal. Package files live flat in the cache directory
//! (`<cache_dir>/name-version-release-arch.{xp,pkg.tar.*}`); this module finds
//! one by reading the metadata of each candidate instead of trusting the file
//! name alone, so a renamed or truncated file is never mistaken for a match.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::XpmResult;
use crate::package::read_metadata;

/// Suffixes recognized as installable package files (mirrors the builders).
pub const PACKAGE_SUFFIXES: &[&str] = &[
    ".pkg.tar.zst",
    ".pkg.tar.xz",
    ".pkg.tar.gz",
    ".pkg.tar.bz2",
    ".pkg.tar",
    ".xp",
];

/// Whether `path` looks like a package file by suffix.
pub fn is_package_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    PACKAGE_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// Finds a cached package whose `.PKGINFO` matches `name` and full `version`
/// (e.g. `0.44.0-1`) exactly. Returns `None` when the cache has no match.
///
/// Unreadable candidates are skipped (they may be partial downloads), so the
/// lookup is resilient instead of failing the whole plan.
pub fn find_package(cache_dir: &Path, name: &str, version: &str) -> XpmResult<Option<PathBuf>> {
    if !cache_dir.is_dir() {
        return Ok(None);
    }

    let prefix = format!("{name}-");
    let mut candidates: Vec<PathBuf> = fs::read_dir(cache_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_package_file(path))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix))
        })
        .collect();
    candidates.sort();

    for path in candidates {
        let metadata = match read_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) => {
                tracing::debug!(
                    path = %path.display(),
                    error = %e,
                    "skipping unreadable cache candidate"
                );
                continue;
            }
        };
        if metadata.meta.name == name && metadata.meta.full_version() == version {
            return Ok(Some(path));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    /// Builds a minimal, valid `.xp` file for `name`/`version`.
    fn write_fake_package(dir: &Path, name: &str, version: &str, release: &str) -> PathBuf {
        let path = dir.join(format!("{name}-{version}-{release}-x86_64.xp"));
        let mut raw_tar = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw_tar);
            let pkginfo =
                format!("pkgname = {name}\npkgver = {version}-{release}\narch = x86_64\n");
            let mut header = tar::Header::new_gnu();
            header.set_path(".PKGINFO").unwrap();
            header.set_size(pkginfo.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, pkginfo.as_bytes()).unwrap();

            let mut header = tar::Header::new_gnu();
            header.set_path(".MTREE").unwrap();
            let mtree = b"#mtree\n./usr type=dir mode=0755 uid=0 gid=0\n";
            header.set_size(mtree.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, &mtree[..]).unwrap();
            builder.finish().unwrap();
        }
        let compressed = zstd::encode_all(&raw_tar[..], 1).expect("compress");
        let mut file = fs::File::create(&path).expect("create package");
        file.write_all(&compressed).expect("write package");
        path
    }

    #[test]
    fn suffix_detection() {
        assert!(is_package_file(Path::new("a-1-1-x86_64.pkg.tar.zst")));
        assert!(is_package_file(Path::new("a-1-1-x86_64.xp")));
        assert!(!is_package_file(Path::new("a-1-1-x86_64.sig")));
        assert!(!is_package_file(Path::new("README.md")));
    }

    #[test]
    fn finds_exact_name_and_version() {
        let tmp = TempDir::new().expect("tmp");
        let wanted = write_fake_package(tmp.path(), "kitty", "0.44.0", "1");
        write_fake_package(tmp.path(), "kitty", "0.45.0", "1");
        write_fake_package(tmp.path(), "other", "0.44.0", "1");

        let found = find_package(tmp.path(), "kitty", "0.44.0-1")
            .expect("lookup")
            .expect("must find");
        assert_eq!(found, wanted);
    }

    #[test]
    fn missing_version_and_missing_dir_are_none() {
        let tmp = TempDir::new().expect("tmp");
        write_fake_package(tmp.path(), "kitty", "0.44.0", "1");

        assert!(find_package(tmp.path(), "kitty", "9.9.9-1")
            .expect("lookup")
            .is_none());
        assert!(find_package(&tmp.path().join("nope"), "kitty", "0.44.0-1")
            .expect("lookup")
            .is_none());
    }

    #[test]
    fn skips_corrupt_candidates() {
        let tmp = TempDir::new().expect("tmp");
        write_fake_package(tmp.path(), "kitty", "0.44.0", "1");
        fs::write(
            tmp.path().join("kitty-0.44.0-1-x86_64.pkg.tar.zst"),
            b"garbage",
        )
        .expect("write");

        let found = find_package(tmp.path(), "kitty", "0.44.0-1").expect("lookup");
        assert!(found.is_some(), "the valid candidate must still be found");
    }
}
