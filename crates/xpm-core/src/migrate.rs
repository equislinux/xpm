//! Import an existing pacman local database into xpm's local database.
//!
//! pacman stores one directory per installed package named
//! `<name>-<version>/` under `/var/lib/pacman/local`, with `desc`, `files`,
//! `mtree` and (optionally) `install` entries. xpm uses
//! `<db_path>/local/<name>/` with its own metadata files. This module bridges
//! the two so an installed system can be adopted without reinstalling every
//! package.
//!
//! The conversion is one-way: pacman's database is never modified.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{XpmError, XpmResult};
use crate::install_reason::InstallReason;
use crate::local_db;
use crate::repo_db::{first_value, parse_alpm_sections};

/// Summary of a migration run.
#[derive(Debug, Default)]
pub struct ImportReport {
    /// Packages written to the xpm local database.
    pub imported: usize,
    /// Packages already present in the destination (skipped without `force`).
    pub skipped: usize,
    /// Packages that could not be imported, with the reason.
    pub failed: Vec<(String, String)>,
}

/// Import every package from `pacman_local` into `dest_local`.
///
/// `force` overwrites existing xpm entries. Packages already present are
/// skipped otherwise. Individual failures do not abort the whole import.
pub fn import_pacman_local(
    pacman_local: &Path,
    dest_local: &Path,
    force: bool,
) -> XpmResult<ImportReport> {
    if !pacman_local.is_dir() {
        return Err(XpmError::Other(format!(
            "pacman local database not found: {}",
            pacman_local.display()
        )));
    }
    if paths_overlap(pacman_local, dest_local) {
        return Err(XpmError::Other(format!(
            "refusing to import '{}' into itself (destination '{}')",
            pacman_local.display(),
            dest_local.display()
        )));
    }

    fs::create_dir_all(dest_local)?;
    let mut report = ImportReport::default();

    let mut entries: Vec<PathBuf> = fs::read_dir(pacman_local)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect();
    entries.sort();

    for dir in entries {
        let desc_path = dir.join("desc");
        if !desc_path.is_file() {
            continue;
        }

        let package_name = dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| dir.display().to_string());

        match import_one(&dir, dest_local, force) {
            Ok(Some(())) => report.imported += 1,
            Ok(None) => report.skipped += 1,
            Err(err) => report.failed.push((package_name, format!("{err:#}"))),
        }
    }

    Ok(report)
}

fn import_one(dir: &Path, dest_local: &Path, force: bool) -> XpmResult<Option<()>> {
    let desc = fs::read_to_string(dir.join("desc"))?;
    let sections = parse_alpm_sections(&desc);

    let name = first_value(&sections, "NAME")
        .ok_or_else(|| XpmError::Other("desc has no %NAME%".to_string()))?;
    let version = first_value(&sections, "VERSION")
        .ok_or_else(|| XpmError::Other("desc has no %VERSION%".to_string()))?;
    if !safe_component(&name) || version.is_empty() || version.contains('/') {
        return Err(XpmError::Other(format!(
            "invalid package identity '{name}' '{version}'"
        )));
    }

    let dest = dest_local.join(&name);
    if dest.exists() && !force {
        return Ok(None);
    }

    // Stage the entry as `<db>/local/.<name>.import-tmp/<name>/` so the
    // local_db helpers (which take a database directory plus package name)
    // can be reused unchanged. The finished directory is renamed into place.
    let staging_root = dest_local.join(format!(".{name}.import-tmp"));
    let staging = staging_root.join(&name);
    if staging_root.exists() {
        fs::remove_dir_all(&staging_root)?;
    }
    fs::create_dir_all(&staging)?;

    let result = (|| -> XpmResult<()> {
        let (files, backups) = read_files(dir, &sections)?;
        local_db::write_file_list(&staging_root, &name, &files)?;
        local_db::write_backup_list(&staging_root, &name, &backups)?;
        local_db::write_depends(
            &staging_root,
            &name,
            &sections.get("DEPENDS").cloned().unwrap_or_default(),
        )?;
        local_db::write_provides(
            &staging_root,
            &name,
            &sections.get("PROVIDES").cloned().unwrap_or_default(),
        )?;
        local_db::write_groups(
            &staging_root,
            &name,
            &sections.get("GROUPS").cloned().unwrap_or_default(),
        )?;
        fs::write(staging.join(local_db::VERSION_FILE), &version)?;

        let reason = match first_value(&sections, "REASON").as_deref() {
            Some("1") => InstallReason::Dep,
            _ => InstallReason::Explicit,
        };
        reason.write(&staging_root, &name)?;

        if let Ok(raw) = fs::read(dir.join("mtree")) {
            local_db::write_mtree(&staging_root, &name, &raw)?;
        }
        if let Ok(install) = fs::read(dir.join("install")) {
            fs::write(staging.join("install"), install)?;
        }
        fs::write(staging.join(local_db::ORIGIN_FILE), "pacman")?;
        Ok(())
    })();

    if let Err(err) = result {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(err);
    }

    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::rename(&staging, &dest)?;
    let _ = fs::remove_dir_all(&staging_root);
    Ok(Some(()))
}

/// Extract the `%FILES%` and `%BACKUP%` sections from a pacman `files` entry.
///
/// `%BACKUP%` lines are `path<TAB>hash`; only the path is kept (the hash is
/// already part of the stored `.MTREE`).
fn read_files(
    dir: &Path,
    desc_sections: &HashMap<String, Vec<String>>,
) -> XpmResult<(Vec<String>, Vec<String>)> {
    let files_path = dir.join("files");
    let sections = if files_path.is_file() {
        Some(parse_alpm_sections(&fs::read_to_string(&files_path)?))
    } else {
        None
    };

    let backups = sections
        .as_ref()
        .and_then(|sections| sections.get("BACKUP"))
        .map(|entries| {
            entries
                .iter()
                .map(|line| {
                    line.split_once('\t')
                        .or_else(|| line.split_once('\0'))
                        .map(|(path, _)| path)
                        .unwrap_or(line)
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .or_else(|| desc_sections.get("BACKUP").cloned())
        .unwrap_or_default();

    // Prefer the stored .MTREE: pacman's `files` list can be stale or carry
    // path-escaping damage from older tools, while the mtree is the package's
    // canonical payload manifest. Fall back to `files` when no usable mtree
    // exists (very old databases).
    if let Ok(raw) = fs::read(dir.join("mtree")) {
        if let Ok(entries) = crate::package::mtree::parse_mtree(&raw) {
            let files = local_db::mtree_paths(&entries);
            if !files.is_empty() {
                return Ok((files, backups));
            }
        }
    }

    if let Some(sections) = sections {
        let files = sections.get("FILES").cloned().unwrap_or_default();
        if !files.is_empty() {
            return Ok((files, backups));
        }
    }

    Ok((Vec::new(), backups))
}

fn safe_component(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains('/')
}

fn paths_overlap(a: &Path, b: &Path) -> bool {
    let normalize = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let a = normalize(a);
    let b = normalize(b);
    a == b || a.starts_with(&b) || b.starts_with(&a)
}

#[cfg(test)]
mod tests;
