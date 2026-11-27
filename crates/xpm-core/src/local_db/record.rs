//! Whole-package local database records.
//!
//! A package entry is made of several files. Writing them one by one leaves a
//! window where `version` already points at the new package while
//! `depends`/`provides` still describe the old one (or the other way around).
//! [`write_record`] writes every metadata file through a temporary file plus a
//! rename, in dependency order, and persists `version` last so it acts as the
//! commit marker for the record.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::XpmResult;
use crate::install_reason::{InstallReason, REASON_FILE};
use crate::package::PackageMeta;

use super::{DEPENDS_FILE, GROUPS_FILE, ORIGIN_FILE, PROVIDES_FILE, VERSION_FILE};

/// Metadata persisted for one installed package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub version: String,
    pub reason: InstallReason,
    pub origin: Option<String>,
    pub depends: Vec<String>,
    pub provides: Vec<String>,
    pub groups: Vec<String>,
}

impl Record {
    /// Builds a record from parsed package metadata.
    pub fn from_meta(meta: &PackageMeta, reason: InstallReason, origin: Option<&str>) -> Self {
        Self {
            name: meta.name.clone(),
            version: meta.full_version(),
            reason,
            origin: origin.map(str::to_string),
            depends: meta.depends.clone(),
            provides: meta.provides.clone(),
            groups: meta.groups.clone(),
        }
    }
}

/// Persists the complete metadata record for a package.
///
/// `version` is written last: a reader that sees the new version can rely on
/// `depends`/`provides`/`groups` being the ones that belong to it.
pub fn write_record(local_db_dir: &Path, record: &Record) -> XpmResult<()> {
    let pkg_dir = local_db_dir.join(&record.name);
    fs::create_dir_all(&pkg_dir)?;

    write_list_atomic(&pkg_dir, DEPENDS_FILE, &record.depends)?;
    write_list_atomic(&pkg_dir, PROVIDES_FILE, &record.provides)?;
    write_list_atomic(&pkg_dir, GROUPS_FILE, &record.groups)?;
    write_atomic(
        &pkg_dir.join(REASON_FILE),
        record.reason.as_str().as_bytes(),
    )?;
    if let Some(origin) = &record.origin {
        write_atomic(&pkg_dir.join(ORIGIN_FILE), origin.as_bytes())?;
    }
    write_atomic(&pkg_dir.join(VERSION_FILE), record.version.as_bytes())?;

    Ok(())
}

fn write_list_atomic(pkg_dir: &Path, file: &str, entries: &[String]) -> XpmResult<()> {
    let mut out = String::new();
    for entry in entries {
        out.push_str(entry);
        out.push('\n');
    }
    write_atomic(&pkg_dir.join(file), out.as_bytes())
}

fn write_atomic(path: &Path, contents: &[u8]) -> XpmResult<()> {
    let tmp = temp_path(path);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("record");
    path.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}
