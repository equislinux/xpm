//! Fast manifest checks and manifest repair.

use super::*;

/// Result of rebuilding one package's `files` manifest from its `.MTREE`.
#[derive(Debug, Clone)]
pub struct ManifestRepair {
    pub package: String,
    /// The manifest differed from the `.MTREE` and was rewritten.
    pub repaired: bool,
    /// No stored `.MTREE`; the existing manifest was left untouched.
    pub skipped: bool,
    pub old_entries: usize,
    pub new_entries: usize,
}

/// Rebuilds a package's `files` manifest from its stored `.MTREE`.
///
/// The package payload and the installed files are not touched. This is the
/// supported recovery path for local databases whose ownership lists were
/// corrupted (for example by a path-escaping bug in an older xpm). Entries
/// such as `@ABS:` shell shims are preserved because they live outside the
/// package payload.
pub fn repair_package_manifest(local_db_dir: &Path, package: &str) -> XpmResult<ManifestRepair> {
    let old = local_db::read_files(local_db_dir, package)?;
    let Some(raw) = local_db::read_mtree(local_db_dir, package)? else {
        return Ok(ManifestRepair {
            package: package.to_string(),
            repaired: false,
            skipped: true,
            old_entries: old.len(),
            new_entries: old.len(),
        });
    };

    let entries = parse_mtree(&raw).map_err(|err| {
        XpmError::Package(format!("invalid stored .MTREE for '{package}': {err}"))
    })?;
    let mut new = local_db::mtree_paths(&entries);
    for path in &old {
        if path.starts_with("@ABS:") && !new.contains(path) {
            new.push(path.clone());
        }
    }

    let mut old_sorted = old.clone();
    old_sorted.sort();
    let mut new_sorted = new.clone();
    new_sorted.sort();
    let repaired = old_sorted != new_sorted;
    if repaired {
        local_db::write_file_list(local_db_dir, package, &new)?;
    }

    Ok(ManifestRepair {
        package: package.to_string(),
        repaired,
        skipped: false,
        old_entries: old.len(),
        new_entries: new.len(),
    })
}

pub(super) fn check_manifest(root: &Path, files: &[String], missing: &mut Vec<String>) {
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
