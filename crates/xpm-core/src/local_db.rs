//! Metadata files stored in each package's local database entry.
//!
//! `<db_path>/local/<pkg>/` holds:
//!
//! - `version` — full package version (`1.2.3-1`).
//! - `reason` — `explicit` or `dep` (see [`crate::install_reason`]).
//! - `files` — pacman-compatible file manifest derived from the package's
//!   `.MTREE`. Starts with the `%FILES%` header and lists relative paths,
//!   directories with a trailing `/`. `x gen restore --pkg` consumes it.
//! - `origin` — name of the repository the package was installed from
//!   (absent for local-file installs).
//! - `depends` — declared runtime dependencies (one per line, raw specs such
//!   as `libc>=2.39`). `None` when absent (legacy install): different from an
//!   empty record, which means the package declares no dependencies.
//! - `provides` — virtual names the package provides, used to resolve which
//!   installed package satisfies a dependency.
//!
//! Missing files never make a legacy install fail: `files` reads as empty
//! and `origin`/`version` as `None`; `depends` as `None` and `provides` as
//! empty.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path};

use crate::error::XpmResult;
use crate::package::types::{MtreeEntry, MtreeFileType};

/// File name of the installed file manifest.
pub const FILES_FILE: &str = "files";
/// File name of the repository-of-origin record.
pub const ORIGIN_FILE: &str = "origin";
/// File name of the installed version.
pub const VERSION_FILE: &str = "version";
/// File name of the declared runtime dependencies.
pub const DEPENDS_FILE: &str = "depends";
/// File name of the provided virtual names.
pub const PROVIDES_FILE: &str = "provides";
/// First line of a pacman-compatible `files` manifest.
pub const FILES_HEADER: &str = "%FILES%";

// ── MTREE conversion ──────────────────────────────────────────

/// Converts `.MTREE` entries into pacman-compatible relative paths.
///
/// The `./` prefix is stripped, directories get a trailing `/` and order is
/// preserved (`.MTREE` is sorted). Duplicates and the root entry are skipped.
pub fn mtree_paths(entries: &[MtreeEntry]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut paths = Vec::with_capacity(entries.len());

    for entry in entries {
        let Some(mut rel) = normalize_mtree_path(&entry.path.to_string_lossy()) else {
            continue;
        };
        if entry.file_type == MtreeFileType::Dir && !rel.ends_with('/') {
            rel.push('/');
        }
        if seen.insert(rel.clone()) {
            paths.push(rel);
        }
    }

    paths
}

fn normalize_mtree_path(raw: &str) -> Option<String> {
    let cleaned = raw.replace('\\', "/");
    let mut parts = Vec::new();

    for component in Path::new(&cleaned).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::RootDir | Component::CurDir => {}
            // Never emit a path that escapes the target root.
            Component::ParentDir => return None,
            Component::Prefix(_) => {}
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("/"))
}

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

// ── version ───────────────────────────────────────────────────

/// Reads the installed version recorded for a package.
pub fn read_version(local_db_dir: &Path, pkg: &str) -> Option<String> {
    fs::read_to_string(local_db_dir.join(pkg).join(VERSION_FILE))
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|value| !value.is_empty())
}

// ── depends / provides ────────────────────────────────────────

fn write_list(local_db_dir: &Path, pkg: &str, file: &str, entries: &[String]) -> XpmResult<()> {
    let pkg_dir = local_db_dir.join(pkg);
    fs::create_dir_all(&pkg_dir)?;
    let mut out = String::new();
    for entry in entries {
        out.push_str(entry);
        out.push('\n');
    }
    fs::write(pkg_dir.join(file), out)?;
    Ok(())
}

fn read_list(local_db_dir: &Path, pkg: &str, file: &str) -> XpmResult<Option<Vec<String>>> {
    match fs::read_to_string(local_db_dir.join(pkg).join(file)) {
        Ok(raw) => Ok(Some(
            raw.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
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

/// Reads the provided virtual names (empty for legacy installs).
pub fn read_provides(local_db_dir: &Path, pkg: &str) -> XpmResult<Vec<String>> {
    Ok(read_list(local_db_dir, pkg, PROVIDES_FILE)?.unwrap_or_default())
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(path: &str, file_type: MtreeFileType) -> MtreeEntry {
        MtreeEntry {
            path: path.into(),
            file_type,
            mode: 0o644,
            uid: 0,
            gid: 0,
            size: 0,
            sha256: None,
            link_target: None,
        }
    }

    fn sample_entries() -> Vec<MtreeEntry> {
        vec![
            entry("./usr", MtreeFileType::Dir),
            entry("./usr/bin", MtreeFileType::Dir),
            entry("./usr/bin/hello", MtreeFileType::File),
            entry("./usr/lib/libfoo.so", MtreeFileType::Link),
            entry("./usr/lib/libfoo.so.1", MtreeFileType::File),
        ]
    }

    #[test]
    fn mtree_paths_include_dirs_files_and_symlinks() {
        let paths = mtree_paths(&sample_entries());
        assert_eq!(
            paths,
            vec![
                "usr/",
                "usr/bin/",
                "usr/bin/hello",
                "usr/lib/libfoo.so",
                "usr/lib/libfoo.so.1",
            ]
        );
    }

    #[test]
    fn mtree_paths_normalize_prefixes_and_skip_root() {
        let entries = vec![
            entry("./", MtreeFileType::Dir),
            entry(".", MtreeFileType::Dir),
            entry("/etc/xpm.conf", MtreeFileType::File),
            entry("usr//bin/./tool", MtreeFileType::File),
            entry("../escape", MtreeFileType::File),
            entry("usr/bin/tool", MtreeFileType::File),
        ];

        let paths = mtree_paths(&entries);
        assert_eq!(paths, vec!["etc/xpm.conf", "usr/bin/tool"]);
    }

    #[test]
    fn write_files_emits_pacman_header() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        write_files(&local_db, "test", &sample_entries()).expect("write files");

        let raw = fs::read_to_string(local_db.join("test").join(FILES_FILE)).expect("read");
        assert_eq!(
            raw,
            "%FILES%\nusr/\nusr/bin/\nusr/bin/hello\nusr/lib/libfoo.so\nusr/lib/libfoo.so.1\n"
        );
    }

    #[test]
    fn read_files_skips_header_and_blank_lines() {
        let tmp = TempDir::new().expect("tmp");
        let pkg_dir = tmp.path().join("local").join("test");
        fs::create_dir_all(&pkg_dir).expect("create pkg dir");
        fs::write(
            pkg_dir.join(FILES_FILE),
            "%FILES%\n\nusr/\nusr/bin/hello\n%sOMETHING%\n",
        )
        .expect("write manifest");

        let files = read_files(tmp.path().join("local").as_path(), "test").expect("read files");
        assert_eq!(files, vec!["usr/", "usr/bin/hello"]);
    }

    #[test]
    fn read_files_roundtrips_written_manifest() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        write_files(&local_db, "test", &sample_entries()).expect("write files");
        let files = read_files(&local_db, "test").expect("read files");
        assert_eq!(files, mtree_paths(&sample_entries()));
    }

    #[test]
    fn read_files_missing_manifest_is_empty() {
        let tmp = TempDir::new().expect("tmp");

        let files = read_files(tmp.path(), "legacy").expect("legacy does not fail");
        assert!(files.is_empty());
    }

    #[test]
    fn origin_roundtrip_and_missing_are_none() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        assert_eq!(read_origin(&local_db, "legacy"), None);

        write_origin(&local_db, "kitty", "x").expect("write origin");
        assert_eq!(read_origin(&local_db, "kitty").as_deref(), Some("x"));

        write_origin(&local_db, "empty", "").expect("write empty origin");
        assert_eq!(read_origin(&local_db, "empty"), None);
    }

    #[test]
    fn depends_roundtrip_and_legacy_is_none() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        assert_eq!(read_depends(&local_db, "legacy").expect("read"), None);

        let deps = vec!["glibc".to_string(), "libx11>=1.8".to_string()];
        write_depends(&local_db, "hello", &deps).expect("write");
        assert_eq!(read_depends(&local_db, "hello").expect("read"), Some(deps));

        write_depends(&local_db, "empty", &[]).expect("write empty");
        assert_eq!(
            read_depends(&local_db, "empty").expect("read"),
            Some(Vec::new()),
            "an empty record is not the same as a missing one"
        );
    }

    #[test]
    fn provides_roundtrip_and_missing_is_empty() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        assert!(read_provides(&local_db, "legacy").expect("read").is_empty());

        write_provides(&local_db, "hello", &["hello-bin".to_string()]).expect("write");
        assert_eq!(
            read_provides(&local_db, "hello").expect("read"),
            vec!["hello-bin".to_string()]
        );
    }

    #[test]
    fn version_roundtrip_and_missing_are_none() {
        let tmp = TempDir::new().expect("tmp");
        let local_db = tmp.path().join("local");

        assert_eq!(read_version(&local_db, "legacy"), None);

        let pkg_dir = local_db.join("kitty");
        fs::create_dir_all(&pkg_dir).expect("create pkg dir");
        fs::write(pkg_dir.join(VERSION_FILE), "0.44.0-1\n").expect("write version");

        assert_eq!(
            read_version(&local_db, "kitty").as_deref(),
            Some("0.44.0-1")
        );
    }
}
