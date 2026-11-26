//! Pacman-compatible `files` manifest rendering and IO.

use super::*;

// ── files ─────────────────────────────────────────────────────

/// Renders a pacman-compatible `files` manifest (`%FILES%` header included).
pub fn render_files(paths: &[String]) -> String {
    let mut out = String::from(FILES_HEADER);
    out.push('\n');
    for path in paths {
        out.push_str(path);
        out.push('\n');
    }
    out
}

/// Writes `paths` as the package's `files` manifest.
pub fn write_file_list(local_db_dir: &Path, pkg: &str, paths: &[String]) -> XpmResult<()> {
    let pkg_dir = local_db_dir.join(pkg);
    fs::create_dir_all(&pkg_dir)?;
    fs::write(pkg_dir.join(FILES_FILE), render_files(paths))?;
    Ok(())
}

/// Writes the package's `files` manifest from its `.MTREE` entries.
pub fn write_files(local_db_dir: &Path, pkg: &str, entries: &[MtreeEntry]) -> XpmResult<()> {
    write_file_list(local_db_dir, pkg, &mtree_paths(entries))
}

/// Reads the package's `files` manifest, skipping header/metadata lines and
/// blanks. A missing manifest (legacy install) reads as an empty list.
pub fn read_files(local_db_dir: &Path, pkg: &str) -> XpmResult<Vec<String>> {
    let path = local_db_dir.join(pkg).join(FILES_FILE);
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    Ok(raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('%'))
        .map(str::to_string)
        .collect())
}
