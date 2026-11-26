//! `.files` database merging.

use super::*;

/// Parse a repository files database archive (`<repo>.files`) and merge file
/// listings into an existing [`SyncDb`].
pub fn merge_files_db(path: &Path, db: &mut SyncDb) -> XpmResult<()> {
    let bytes = fs::read(path)?;
    merge_files_db_bytes(&bytes, db)
}

pub(super) fn merge_files_db_bytes(bytes: &[u8], db: &mut SyncDb) -> XpmResult<()> {
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
