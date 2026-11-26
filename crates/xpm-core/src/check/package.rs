//! Package-level check orchestration.

use super::*;

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
