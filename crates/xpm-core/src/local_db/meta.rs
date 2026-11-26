//! Origin, version, changelog, dependencies, provides and groups.

use super::*;

// ── origin ────────────────────────────────────────────────────

/// Records the repository a package was installed from.
pub fn write_origin(local_db_dir: &Path, pkg: &str, origin: &str) -> XpmResult<()> {
    let pkg_dir = local_db_dir.join(pkg);
    fs::create_dir_all(&pkg_dir)?;
    fs::write(pkg_dir.join(ORIGIN_FILE), origin)?;
    Ok(())
}

/// Reads the repository a package was installed from. A missing or empty
/// record (legacy install, local file) yields `None`.
pub fn read_origin(local_db_dir: &Path, pkg: &str) -> Option<String> {
    fs::read_to_string(local_db_dir.join(pkg).join(ORIGIN_FILE))
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ── changelog ─────────────────────────────────────────────────

/// Reads the changelog stored for an installed package.
///
/// pacman keeps a `changelog` file in the local database when the package
/// ships one; packages without it return `None`, matching `pacman -Qc`.
pub fn read_changelog(local_db_dir: &Path, pkg: &str) -> XpmResult<Option<String>> {
    match fs::read_to_string(local_db_dir.join(pkg).join(CHANGELOG_FILE)) {
        Ok(raw) => Ok(Some(raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

// ── version ───────────────────────────────────────────────────

/// Reads the installed version recorded for a package.
pub fn read_version(local_db_dir: &Path, pkg: &str) -> Option<String> {
    fs::read_to_string(local_db_dir.join(pkg).join(VERSION_FILE))
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Writes the package's declared runtime dependencies (raw specs).
pub fn write_depends(local_db_dir: &Path, pkg: &str, depends: &[String]) -> XpmResult<()> {
    write_list(local_db_dir, pkg, DEPENDS_FILE, depends)
}

/// Reads the declared dependencies. `None` for a legacy install without a
/// record; `Some(vec![])` when the package explicitly declares none.
pub fn read_depends(local_db_dir: &Path, pkg: &str) -> XpmResult<Option<Vec<String>>> {
    read_list(local_db_dir, pkg, DEPENDS_FILE)
}

/// Writes the virtual names the package provides.
pub fn write_provides(local_db_dir: &Path, pkg: &str, provides: &[String]) -> XpmResult<()> {
    write_list(local_db_dir, pkg, PROVIDES_FILE, provides)
}

/// Writes the package groups.
pub fn write_groups(local_db_dir: &Path, pkg: &str, groups: &[String]) -> XpmResult<()> {
    write_list(local_db_dir, pkg, GROUPS_FILE, groups)
}

/// Reads the package groups (empty for legacy installs).
pub fn read_groups(local_db_dir: &Path, pkg: &str) -> XpmResult<Vec<String>> {
    Ok(read_list(local_db_dir, pkg, GROUPS_FILE)?.unwrap_or_default())
}

/// Reads the provided virtual names (empty for legacy installs).
pub fn read_provides(local_db_dir: &Path, pkg: &str) -> XpmResult<Vec<String>> {
    Ok(read_list(local_db_dir, pkg, PROVIDES_FILE)?.unwrap_or_default())
}
