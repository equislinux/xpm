//! Raw metadata entry access.

use super::*;

// ── Raw entry reader ──────────────────────────────────────────

/// Read a specific metadata entry from a package archive by name.
///
/// Returns `None` if the entry does not exist.
pub fn read_raw_entry(path: &Path, entry_name: &str) -> Result<Option<Vec<u8>>, XpmError> {
    let mut archive = open_archive(path)?;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.to_path_buf();
        let name = entry_path.to_string_lossy();

        // Match with or without leading "./"
        if name == entry_name || name == format!("./{entry_name}") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            return Ok(Some(buf));
        }
    }

    Ok(None)
}
