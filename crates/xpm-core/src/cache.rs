//! Local package-cache lookup.
//!
//! `xpm rollback` needs the *old* package file to undo an upgrade or a
//! removal. Package files live flat in the cache directory
//! (`<cache_dir>/name-version-release-arch.{xp,pkg.tar.*}`); this module finds
//! one by reading the metadata of each candidate instead of trusting the file
//! name alone, so a renamed or truncated file is never mistaken for a match.

use std::collections::HashMap;
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

/// Result of [`clean_cache`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CacheCleanReport {
    /// Package files that were removed.
    pub removed: Vec<PathBuf>,
    /// Package files that were kept.
    pub kept: usize,
    /// Total size of the removed package files.
    pub bytes_freed: u64,
}

/// Removes cached package files.
///
/// With `all = false` (`pacman -Sc`), only package files whose metadata does
/// not match an installed name/version are removed. Unreadable candidates are
/// kept because their identity cannot be established safely. With
/// `all = true` (`pacman -Scc`), every recognized package file is removed.
///
/// Only files with a known package suffix are touched; signatures belonging
/// to removed packages are removed as well. Unrelated cache files are left
/// alone. Sync databases live in the database directory and are not part of
/// this operation.
pub fn clean_cache(
    cache_dir: &Path,
    installed: &HashMap<String, String>,
    all: bool,
) -> XpmResult<CacheCleanReport> {
    let mut report = CacheCleanReport::default();
    if !cache_dir.is_dir() {
        return Ok(report);
    }

    let mut candidates: Vec<PathBuf> = fs::read_dir(cache_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_package_file(path))
        .collect();
    candidates.sort();

    for path in candidates {
        let remove = if all {
            true
        } else {
            match read_metadata(&path) {
                Ok(metadata) => !installed
                    .get(&metadata.meta.name)
                    .is_some_and(|version| version == &metadata.meta.full_version()),
                Err(e) => {
                    tracing::debug!(
                        path = %path.display(),
                        error = %e,
                        "keeping unreadable cache candidate"
                    );
                    false
                }
            }
        };

        if !remove {
            report.kept += 1;
            continue;
        }

        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        fs::remove_file(&path)?;
        let signature = PathBuf::from(format!("{}.sig", path.display()));
        if signature.is_file() {
            let _ = fs::remove_file(&signature);
        }
        report.bytes_freed += size;
        report.removed.push(path);
    }

    Ok(report)
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

    #[test]
    fn clean_cache_keeps_installed_versions_and_drops_the_rest() {
        let tmp = TempDir::new().expect("tmp");
        let current = write_fake_package(tmp.path(), "kitty", "0.44.0", "1");
        let old = write_fake_package(tmp.path(), "kitty", "0.43.0", "1");
        let orphan = write_fake_package(tmp.path(), "orphan", "2.0.0", "1");
        fs::write(
            tmp.path().join("kitty-0.44.0-1-x86_64.xp.sig"),
            b"signature",
        )
        .expect("sig");
        fs::write(tmp.path().join("notes.txt"), b"keep me").expect("notes");

        let installed = HashMap::from([("kitty".to_string(), "0.44.0-1".to_string())]);
        let report = clean_cache(tmp.path(), &installed, false).expect("clean");

        assert!(current.is_file(), "the installed version stays cached");
        assert!(!old.exists());
        assert!(!orphan.exists());
        assert!(current
            .with_file_name("kitty-0.44.0-1-x86_64.xp.sig")
            .is_file());
        assert!(tmp.path().join("notes.txt").is_file());
        assert_eq!(report.kept, 1);
        assert_eq!(report.removed.len(), 2);
        assert!(report.bytes_freed > 0);
    }

    #[test]
    fn clean_cache_all_removes_package_artifacts_including_corrupt_ones() {
        let tmp = TempDir::new().expect("tmp");
        write_fake_package(tmp.path(), "kitty", "0.44.0", "1");
        fs::write(
            tmp.path().join("broken-1.0-1-x86_64.pkg.tar.zst"),
            b"garbage",
        )
        .expect("corrupt package");

        let report = clean_cache(tmp.path(), &HashMap::new(), true).expect("clean everything");

        assert_eq!(report.removed.len(), 2);
        assert!(fs::read_dir(tmp.path()).expect("dir").next().is_none());
    }

    #[test]
    fn clean_cache_safe_mode_keeps_corrupt_and_unrelated_files() {
        let tmp = TempDir::new().expect("tmp");
        fs::write(
            tmp.path().join("broken-1.0-1-x86_64.pkg.tar.zst"),
            b"garbage",
        )
        .expect("corrupt package");
        fs::write(tmp.path().join("README"), b"notes").expect("notes");

        let report = clean_cache(tmp.path(), &HashMap::new(), false).expect("clean");

        assert!(report.removed.is_empty());
        assert_eq!(report.kept, 1);
        assert!(tmp.path().join("README").is_file());
    }
}
