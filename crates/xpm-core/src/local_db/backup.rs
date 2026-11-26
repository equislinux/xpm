//! Backup-file lists and the stored `.MTREE` copy.

use super::*;

/// Writes the package's configuration-file list (`backup` entries).
pub fn write_backup_list(local_db_dir: &Path, pkg: &str, paths: &[String]) -> XpmResult<()> {
    write_list(local_db_dir, pkg, BACKUP_FILE, paths)
}

/// Reads the package's configuration-file list (empty for legacy installs).
pub fn read_backup_list(local_db_dir: &Path, pkg: &str) -> XpmResult<Vec<String>> {
    Ok(read_list(local_db_dir, pkg, BACKUP_FILE)?.unwrap_or_default())
}

/// Stores the raw `.MTREE` of the installed package (plain or gzip-wrapped,
/// exactly as it was inside the archive).
pub fn write_mtree(local_db_dir: &Path, pkg: &str, raw: &[u8]) -> XpmResult<()> {
    let pkg_dir = local_db_dir.join(pkg);
    fs::create_dir_all(&pkg_dir)?;
    fs::write(pkg_dir.join(MTREE_FILE), raw)?;
    Ok(())
}

/// Reads the stored `.MTREE` bytes, if any.
pub fn read_mtree(local_db_dir: &Path, pkg: &str) -> XpmResult<Option<Vec<u8>>> {
    match fs::read(local_db_dir.join(pkg).join(MTREE_FILE)) {
        Ok(raw) => Ok(Some(raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
