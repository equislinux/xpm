//! Package payload extraction.

use super::*;

// ── Extract ───────────────────────────────────────────────────

/// Extract all package files (excluding metadata) to a destination directory.
pub fn extract_to(pkg_path: &Path, dest: &Path) -> Result<Vec<PathBuf>, XpmError> {
    let mut archive = open_archive(pkg_path)?;
    let mut extracted = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.to_path_buf();
        let name = entry_path.to_string_lossy().to_string();
        let normalized = name.strip_prefix("./").unwrap_or(&name);

        // Skip metadata dot-files.
        if normalized.starts_with('.') {
            continue;
        }

        let target = dest.join(normalized);
        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
        }
        extracted.push(PathBuf::from(normalized));
    }

    Ok(extracted)
}
