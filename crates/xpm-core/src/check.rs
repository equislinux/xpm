//! Installed-file verification for `xpm check` (pacman `-Qk` / `-Qkk`).
//!
//! The fast pass verifies that every path in the package's `files` manifest
//! still exists with the recorded file/directory type. The full pass also
//! parses the stored `.MTREE` copy and verifies regular-file SHA-256 digests,
//! symlink targets, and basic mode/type information. Configuration files
//! recorded in the local-database `backup` list are allowed to differ.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{XpmError, XpmResult};
use crate::local_db;
use crate::package::mtree::parse_mtree;
use crate::package::types::{MtreeEntry, MtreeFileType};

/// Package metadata entries that must never be checked as package payload.
const METADATA_PATHS: &[&str] = &[".PKGINFO", ".BUILDINFO", ".MTREE", ".INSTALL"];

/// Result of checking one installed package.
#[derive(Debug, Clone, Default)]
pub struct PackageCheck {
    pub package: String,
    /// Number of entries in the package's `files` manifest.
    pub total: usize,
    /// Manifest entries that are missing or have the wrong file/dir type.
    pub missing: Vec<String>,
    /// `.MTREE` paths whose digest, target or mode no longer matches.
    pub altered: Vec<String>,
    /// The package has no stored `.MTREE`, so the full pass could not run.
    pub metadata_missing: bool,
    /// Individual `.MTREE` entries without enough metadata to verify.
    pub metadata_issues: Vec<String>,
}

impl PackageCheck {
    /// Whether the check passed. Altered files and metadata only matter in
    /// full mode.
    pub fn is_ok(&self, full: bool) -> bool {
        self.missing.is_empty()
            && (!full || {
                self.altered.is_empty() && !self.metadata_missing && self.metadata_issues.is_empty()
            })
    }
}

/// Checks an installed package against `root`.
///
/// `local_db_dir` is the xpm local database; `package` must be an installed
/// package name. Fast mode only needs the `files` manifest. Full mode also
/// uses the stored `mtree` and `backup` records.
pub fn check_package(
    root: &Path,
    local_db_dir: &Path,
    package: &str,
    full: bool,
) -> XpmResult<PackageCheck> {
    let files = local_db::read_files(local_db_dir, package)?;
    let mut report = PackageCheck {
        package: package.to_string(),
        total: files.len(),
        ..PackageCheck::default()
    };

    check_manifest(root, &files, &mut report.missing);

    if full {
        check_mtree(root, local_db_dir, package, &files, &mut report)?;
    }

    report.missing.sort();
    report.missing.dedup();
    report.altered.sort();
    report.altered.dedup();
    report.metadata_issues.sort();
    report.metadata_issues.dedup();

    Ok(report)
}

fn check_manifest(root: &Path, files: &[String], missing: &mut Vec<String>) {
    for entry in files {
        let expected_dir = entry.ends_with('/');
        let Some(relative) = normalize_entry(entry) else {
            continue;
        };
        let path = root.join(&relative);

        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                let is_dir = metadata.file_type().is_dir();
                if is_dir != expected_dir {
                    missing.push(entry.clone());
                }
            }
            Err(_) => missing.push(entry.clone()),
        }
    }
}

fn check_mtree(
    root: &Path,
    local_db_dir: &Path,
    package: &str,
    manifest: &[String],
    report: &mut PackageCheck,
) -> XpmResult<()> {
    let backups: HashSet<String> = local_db::read_backup_list(local_db_dir, package)?
        .iter()
        .filter_map(|entry| normalize_entry(entry))
        .collect();

    let Some(raw) = local_db::read_mtree(local_db_dir, package)? else {
        report.metadata_missing = true;
        return Ok(());
    };

    let entries = parse_mtree(&raw).map_err(|err| {
        XpmError::Package(format!("invalid stored .MTREE for '{package}': {err}"))
    })?;

    let mut known = HashSet::new();
    for entry in entries {
        let Some(relative) = normalize_entry(&entry.path.to_string_lossy()) else {
            continue;
        };
        if METADATA_PATHS.contains(&relative.as_str()) {
            continue;
        }
        known.insert(relative.clone());
        verify_entry(root, &entry, relative, &backups, report)?;
    }

    // Manifest entries without a matching .MTREE record cannot be verified.
    for entry in manifest {
        let Some(relative) = normalize_entry(entry) else {
            continue;
        };
        if !known.contains(&relative) {
            report.metadata_issues.push(relative);
        }
    }

    Ok(())
}

fn verify_entry(
    root: &Path,
    entry: &MtreeEntry,
    relative: String,
    backups: &HashSet<String>,
    report: &mut PackageCheck,
) -> XpmResult<()> {
    let is_backup = backups.contains(&relative);
    let path = root.join(&relative);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            report.missing.push(relative);
            return Ok(());
        }
        Err(err) => {
            return Err(XpmError::Io(err));
        }
    };

    let actual_type = if metadata.file_type().is_dir() {
        MtreeFileType::Dir
    } else if metadata.file_type().is_symlink() {
        MtreeFileType::Link
    } else {
        MtreeFileType::File
    };

    if actual_type != entry.file_type {
        report.altered.push(relative);
        return Ok(());
    }

    match entry.file_type {
        MtreeFileType::Dir => {
            if !is_backup && mode(&metadata) != entry.mode & 0o7777 {
                report.altered.push(relative);
            }
        }
        MtreeFileType::File => {
            if !is_backup && mode(&metadata) != entry.mode & 0o7777 {
                report.altered.push(relative.clone());
            }

            match entry.sha256.as_deref() {
                Some(expected) if !is_backup => {
                    let actual = sha256_file(&path)?;
                    if !actual.eq_ignore_ascii_case(expected) {
                        report.altered.push(relative);
                    }
                }
                Some(_) => {}
                None => report.metadata_issues.push(relative),
            }
        }
        MtreeFileType::Link => match entry.link_target.as_deref() {
            Some(expected) => {
                let actual = fs::read_link(&path)?;
                if actual.to_string_lossy() != expected {
                    report.altered.push(relative);
                }
            }
            None => report.metadata_issues.push(relative),
        },
    }

    Ok(())
}

#[cfg(unix)]
fn mode(metadata: &fs::Metadata) -> u32 {
    metadata.permissions().mode() & 0o7777
}

fn normalize_entry(raw: &str) -> Option<String> {
    let mut parts = Vec::new();
    for component in Path::new(raw).components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => return None,
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn sha256_file(path: &Path) -> XpmResult<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn sha256_hex(data: &[u8]) -> String {
        Sha256::digest(data)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn write_file(root: &Path, relative: &str, data: &[u8], mode: u32) -> fs::Metadata {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        let mut file = File::create(&path).expect("create");
        file.write_all(data).expect("write");
        drop(file);
        let mut permissions = fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(mode);
        fs::set_permissions(&path, permissions).expect("chmod");
        fs::metadata(&path).expect("metadata")
    }

    fn write_mtree(local_db: &Path, package: &str, raw: &str) {
        local_db::write_mtree(local_db, package, raw.as_bytes()).expect("mtree");
    }

    #[test]
    fn fast_check_reports_missing_and_type_mismatches() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let local_db = tmp.path().join("local");
        let pkg_dir = local_db.join("hello");
        fs::create_dir_all(&pkg_dir).expect("db");
        fs::write(pkg_dir.join("version"), "1.0-1").expect("version");

        fs::create_dir_all(root.join("usr/bin")).expect("dirs");
        write_file(&root, "usr/bin/hello", b"hello", 0o755);
        fs::create_dir_all(root.join("usr/share")).expect("share");

        local_db::write_file_list(
            &local_db,
            "hello",
            &[
                "usr/".to_string(),
                "usr/bin/".to_string(),
                "usr/bin/hello".to_string(),
                "usr/share/".to_string(),
            ],
        )
        .expect("files");

        let report = check_package(&root, &local_db, "hello", false).expect("check");
        assert_eq!(report.total, 4);
        assert!(report.missing.is_empty());
        assert!(report.is_ok(false));

        fs::remove_file(root.join("usr/bin/hello")).expect("remove");
        fs::remove_dir(root.join("usr/share")).expect("remove dir");
        write_file(&root, "usr/share", b"now a file", 0o644);

        let report = check_package(&root, &local_db, "hello", false).expect("check");
        assert_eq!(report.missing.len(), 2);
        assert!(report.missing.contains(&"usr/bin/hello".to_string()));
        assert!(report.missing.contains(&"usr/share/".to_string()));
        assert!(!report.is_ok(false));
    }

    #[test]
    fn full_check_verifies_hashes_modes_and_symlinks() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let local_db = tmp.path().join("local");
        let pkg_dir = local_db.join("hello");
        fs::create_dir_all(&pkg_dir).expect("db");
        fs::write(pkg_dir.join("version"), "1.0-1").expect("version");

        let binary = b"#!/bin/sh\necho hello\n";
        write_file(&root, "usr/bin/hello", binary, 0o755);
        fs::create_dir_all(root.join("usr/bin")).expect("bin dir");
        let target = root.join("usr/bin/hello-real");
        std::os::unix::fs::symlink("hello", &target).expect("symlink");

        let binary_hash = sha256_hex(binary);
        let mtree = format!(
            "#mtree\n\
             ./usr type=dir mode=0755 uid=0 gid=0\n\
             ./usr/bin type=dir mode=0755 uid=0 gid=0\n\
             ./usr/bin/hello type=file mode=0755 size={} sha256digest={binary_hash} uid=0 gid=0\n\
             ./usr/bin/hello-real type=link link=hello uid=0 gid=0\n",
            binary.len()
        );
        write_mtree(&local_db, "hello", &mtree);
        local_db::write_file_list(
            &local_db,
            "hello",
            &[
                "usr/".to_string(),
                "usr/bin/".to_string(),
                "usr/bin/hello".to_string(),
                "usr/bin/hello-real".to_string(),
            ],
        )
        .expect("files");

        let report = check_package(&root, &local_db, "hello", true).expect("check");
        assert!(report.altered.is_empty(), "{report:?}");
        assert!(!report.metadata_missing);
        assert!(report.is_ok(true));

        write_file(&root, "usr/bin/hello", b"tampered", 0o755);
        let report = check_package(&root, &local_db, "hello", true).expect("check");
        assert_eq!(report.altered, vec!["usr/bin/hello".to_string()]);
        assert!(!report.is_ok(true));

        write_file(&root, "usr/bin/hello", binary, 0o644);
        let report = check_package(&root, &local_db, "hello", true).expect("check");
        assert_eq!(report.altered, vec!["usr/bin/hello".to_string()]);

        fs::remove_file(&target).expect("remove link");
        std::os::unix::fs::symlink("wrong-target", &target).expect("symlink");
        let report = check_package(&root, &local_db, "hello", true).expect("check");
        assert!(report.altered.contains(&"usr/bin/hello-real".to_string()));
    }

    #[test]
    fn backup_changes_are_not_fatal() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let local_db = tmp.path().join("local");
        let pkg_dir = local_db.join("hello");
        fs::create_dir_all(&pkg_dir).expect("db");
        fs::write(pkg_dir.join("version"), "1.0-1").expect("version");

        write_file(&root, "etc/hello.conf", b"default", 0o644);
        let hash = sha256_hex(b"default");
        write_mtree(
            &local_db,
            "hello",
            &format!(
                "#mtree\n./etc/hello.conf type=file mode=0644 size=7 sha256digest={hash} uid=0 gid=0\n"
            ),
        );
        local_db::write_file_list(&local_db, "hello", &["etc/hello.conf".to_string()])
            .expect("files");
        local_db::write_backup_list(&local_db, "hello", &["etc/hello.conf".to_string()])
            .expect("backup");

        write_file(&root, "etc/hello.conf", b"edited by the user", 0o600);
        let report = check_package(&root, &local_db, "hello", true).expect("check");
        assert!(report.altered.is_empty(), "{report:?}");
        assert!(report.is_ok(true));
    }

    #[test]
    fn missing_mtree_is_reported_in_full_mode_only() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let local_db = tmp.path().join("local");
        fs::create_dir_all(&local_db).expect("db");
        fs::create_dir_all(&root).expect("root");

        let fast = check_package(&root, &local_db, "legacy", false).expect("fast");
        assert_eq!(fast.total, 0);
        assert!(fast.is_ok(false));

        let full = check_package(&root, &local_db, "legacy", true).expect("full");
        assert!(full.metadata_missing);
        assert!(!full.is_ok(true));
    }

    #[test]
    fn manifest_entries_without_mtree_records_are_metadata_issues() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let local_db = tmp.path().join("local");
        fs::create_dir_all(root.join("usr")).expect("dirs");
        write_file(&root, "usr/tracked", b"tracked", 0o644);
        write_file(&root, "usr/untracked", b"untracked", 0o644);
        let hash = sha256_hex(b"tracked");
        write_mtree(
            &local_db,
            "hello",
            &format!(
                "#mtree\n./usr/tracked type=file mode=0644 size=7 sha256digest={hash} uid=0 gid=0\n"
            ),
        );
        local_db::write_file_list(
            &local_db,
            "hello",
            &["usr/tracked".to_string(), "usr/untracked".to_string()],
        )
        .expect("files");

        let report = check_package(&root, &local_db, "hello", true).expect("check");
        assert_eq!(report.metadata_issues, vec!["usr/untracked".to_string()]);
        assert!(!report.is_ok(true));
    }

    #[test]
    fn missing_hashes_are_reported_as_metadata_issues() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let local_db = tmp.path().join("local");
        fs::create_dir_all(root.join("usr")).expect("dirs");
        write_file(&root, "usr/data", b"data", 0o644);
        write_mtree(
            &local_db,
            "hello",
            "#mtree\n./usr/data type=file mode=0644 size=4 uid=0 gid=0\n",
        );
        local_db::write_file_list(&local_db, "hello", &["usr/data".to_string()]).expect("files");

        let report = check_package(&root, &local_db, "hello", true).expect("check");
        assert_eq!(report.metadata_issues, vec!["usr/data".to_string()]);
        assert!(!report.is_ok(true));
    }
}
