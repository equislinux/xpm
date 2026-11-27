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

use std::collections::{HashMap, HashSet};
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
/// File name of the package groups.
pub const GROUPS_FILE: &str = "groups";
/// File name of the configuration-file list (`.PKGINFO` `backup` entries).
pub const BACKUP_FILE: &str = "backup";
/// File name of the package changelog stored by pacman-compatible databases.
pub const CHANGELOG_FILE: &str = "changelog";
/// File name of the raw `.MTREE` copy, kept for integrity and for deciding
/// whether a configuration file was modified (`.pacnew`/`.pacsave`).
pub const MTREE_FILE: &str = "mtree";
/// First line of a pacman-compatible `files` manifest.
pub const FILES_HEADER: &str = "%FILES%";

mod backup;
mod files;
mod meta;
mod mtree;
mod record;

pub use backup::{read_backup_list, read_mtree, write_backup_list, write_mtree};
pub use files::{read_files, render_files, write_file_list, write_files};
pub use meta::{
    read_changelog, read_depends, read_groups, read_origin, read_provides, read_version,
    write_depends, write_groups, write_origin, write_provides,
};
pub use mtree::{mtree_hash_map, mtree_paths};
pub use record::{write_record, Record};

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

#[cfg(test)]
mod tests;
