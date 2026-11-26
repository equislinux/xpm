//! Package file listing.

use super::*;

// ── File listing ──────────────────────────────────────────────

/// List all non-metadata files inside a package archive.
pub fn list_files(path: &Path) -> Result<Vec<PackageFile>, XpmError> {
    let mut archive = open_archive(path)?;
    let mut files = Vec::new();

    for entry in archive.entries()? {
        let entry = entry?;
        let entry_path = entry.path()?.to_path_buf();
        let name = entry_path.to_string_lossy().to_string();

        let normalized = name.strip_prefix("./").unwrap_or(&name);

        // Skip metadata dot-files.
        if normalized.starts_with('.') {
            continue;
        }

        files.push(PackageFile {
            path: PathBuf::from(normalized),
            size: entry.size(),
            is_dir: entry.header().entry_type().is_dir(),
        });
    }

    Ok(files)
}
