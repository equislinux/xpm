//! Write pacman-compatible entries into a pacman local database.
//!
//! xpm keeps its own local database under `<db_path>/local` (one directory per
//! package name). pacman expects `<pacman_local>/<name>-<version>/` with
//! `desc`, `files` and optional `mtree`/`install` files. Mirroring an xpm
//! install into pacman's database keeps `pacman -Q`, `-Qi`, `-Ql`, `-Qk`,
//! `-Syu` and `-R` consistent with packages installed through xpm.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};

use crate::error::{XpmError, XpmResult};
use crate::install_reason::InstallReason;
use crate::local_db;
use crate::package::read_metadata;
use crate::package::reader::read_raw_entry;
use crate::package::PackageMeta;

const ALPM_DB_VERSION: &str = "ALPM_DB_VERSION";
const ALPM_DB_VERSION_SUPPORTED: u32 = 9;
const DB_LOCK: &str = "db.lck";
const STAGING_DIR: &str = ".xpm-staging";

/// Summary of a [`sync_all`] run.
#[derive(Debug, Default)]
pub struct SyncReport {
    pub written: usize,
    pub skipped: usize,
    pub failed: Vec<(String, String)>,
}

/// Mirror one installed package into pacman's local database.
///
/// The entry is staged outside `pacman_local`, the pacman database lock is
/// acquired, and all existing entries for `name` are replaced atomically.
pub fn mirror_entry(
    pacman_local: &Path,
    xpm_local: &Path,
    name: &str,
    version: &str,
    reason: InstallReason,
    pkg_file: Option<&Path>,
    root_dir: &Path,
) -> XpmResult<()> {
    validate_package_name(name)?;
    let full_version = resolve_version(xpm_local, name, version, pkg_file)?;
    validate_version(&full_version)?;
    let entry_name = format!("{name}-{full_version}");

    ensure_db_version(pacman_local)?;

    let dbpath = db_path(pacman_local)?;
    let staging_root = dbpath.join(STAGING_DIR);
    let staging = staging_root.join(format!("{}-{entry_name}", std::process::id()));

    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    create_dir_mode(&staging, 0o755)?;

    if let Err(err) = write_entry_contents(
        &staging,
        xpm_local,
        name,
        &full_version,
        reason,
        pkg_file,
        root_dir,
    ) {
        cleanup_staging(&staging);
        return Err(err);
    }
    sync_dir(&staging);

    let lock = match DbLock::acquire(&dbpath) {
        Ok(lock) => lock,
        Err(err) => {
            cleanup_staging(&staging);
            return Err(err);
        }
    };

    let result = (|| -> XpmResult<()> {
        remove_matching_dirs(pacman_local, name)?;
        let dest = pacman_local.join(&entry_name);
        if dest.exists() {
            fs::remove_dir_all(&dest)?;
        }
        fs::rename(&staging, &dest)?;
        sync_dir(pacman_local);
        Ok(())
    })();

    drop(lock);
    cleanup_staging(&staging);
    result
}

/// Remove every pacman local entry whose parsed package name equals `name`.
pub fn remove_entry(pacman_local: &Path, name: &str) -> XpmResult<()> {
    validate_package_name(name)?;
    ensure_db_version(pacman_local)?;

    let dbpath = db_path(pacman_local)?;
    let _lock = DbLock::acquire(&dbpath)?;

    remove_matching_dirs(pacman_local, name)?;
    sync_dir(pacman_local);
    Ok(())
}

/// Mirror every package recorded in the xpm local database.
///
/// Cached package archives are looked up in `cache_dir` by
/// `<name>-<version>-*`. Missing archives fall back to a minimal `desc`; a
/// package with backup files but no archive is reported as failed because its
/// original configuration hashes cannot be recovered reliably.
pub fn sync_all(
    pacman_local: &Path,
    xpm_local: &Path,
    cache_dir: &Path,
    root_dir: &Path,
) -> XpmResult<SyncReport> {
    let mut report = SyncReport::default();

    let entries = match fs::read_dir(xpm_local) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(err) => return Err(err.into()),
    };

    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .collect();
    dirs.sort();

    for dir in dirs {
        let file_name = dir.file_name();
        let Some(name) = file_name.and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            report.skipped += 1;
            continue;
        }

        let Some(version) = local_db::read_version(xpm_local, name) else {
            report.skipped += 1;
            continue;
        };
        let reason = InstallReason::read(xpm_local, name);
        if pacman_entry_exists(pacman_local, name, &version) {
            report.skipped += 1;
            continue;
        }
        let cached = find_cached_package(cache_dir, name, &version);

        if cached.is_none() {
            match local_db::read_backup_list(xpm_local, name) {
                Ok(backups) if !backups.is_empty() => {
                    report.failed.push((
                        name.to_string(),
                        format!(
                            "no cached package file for {name}-{version} and the package has {} backup file(s)",
                            backups.len()
                        ),
                    ));
                    continue;
                }
                Ok(_) => {}
                Err(err) => {
                    report.failed.push((name.to_string(), format!("{err:#}")));
                    continue;
                }
            }
        }

        match mirror_entry(
            pacman_local,
            xpm_local,
            name,
            &version,
            reason,
            cached.as_deref(),
            root_dir,
        ) {
            Ok(()) => report.written += 1,
            Err(err) => report.failed.push((name.to_string(), format!("{err:#}"))),
        }
    }

    Ok(report)
}

fn write_entry_contents(
    staging: &Path,
    xpm_local: &Path,
    name: &str,
    version: &str,
    reason: InstallReason,
    pkg_file: Option<&Path>,
    root_dir: &Path,
) -> XpmResult<()> {
    let meta = match pkg_file {
        Some(path) => Some(read_metadata(path)?.meta),
        None => None,
    };

    let mtree = local_db::read_mtree(xpm_local, name)?;
    let size_fallback = mtree
        .as_deref()
        .and_then(|raw| crate::package::mtree::parse_mtree(raw).ok())
        .map(|entries| entries.iter().map(|entry| entry.size).sum::<u64>())
        .filter(|size| *size > 0);

    let depends = match &meta {
        Some(meta) => meta.depends.clone(),
        None => local_db::read_depends(xpm_local, name)?.unwrap_or_default(),
    };
    let provides = match &meta {
        Some(meta) => meta.provides.clone(),
        None => local_db::read_provides(xpm_local, name)?,
    };

    let desc = render_desc(
        name,
        version,
        reason,
        meta.as_ref(),
        &depends,
        &provides,
        size_fallback,
    );
    write_file_mode(&staging.join("desc"), desc.as_bytes())?;

    let files = collect_files(xpm_local, name)?;
    let backups = collect_backup_hashes(xpm_local, name, meta.as_ref(), pkg_file, root_dir)?;
    write_file_mode(
        &staging.join("files"),
        render_files(&files, &backups).as_bytes(),
    )?;

    if let Some(raw) = mtree {
        write_file_mode(&staging.join("mtree"), &raw)?;
    }

    match fs::read(xpm_local.join(name).join("install")) {
        Ok(install) => write_file_mode(&staging.join("install"), &install)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }

    Ok(())
}

fn render_desc(
    name: &str,
    version: &str,
    reason: InstallReason,
    meta: Option<&PackageMeta>,
    depends: &[String],
    provides: &[String],
    size_fallback: Option<u64>,
) -> String {
    let mut desc = String::new();

    push_one(&mut desc, "NAME", name);
    push_one(&mut desc, "VERSION", version);
    let base = meta
        .and_then(|meta| extra_first(meta, "pkgbase"))
        .unwrap_or_else(|| name.to_string());
    push_one(&mut desc, "BASE", &base);

    if let Some(meta) = meta {
        push_one(&mut desc, "DESC", &meta.description);
        push_one(&mut desc, "URL", &meta.url);
        if let Some(arch) = meta.arch.iter().find(|arch| !arch.trim().is_empty()) {
            push_one(&mut desc, "ARCH", arch);
        }
        if meta.build_date > 0 {
            push_one(&mut desc, "BUILDDATE", &meta.build_date.to_string());
        }
    }

    push_one(&mut desc, "INSTALLDATE", &install_date().to_string());

    if let Some(packager) = meta.and_then(|meta| extra_first(meta, "packager")) {
        push_one(&mut desc, "PACKAGER", &packager);
    }

    let size = meta
        .map(|meta| meta.installed_size)
        .filter(|size| *size > 0)
        .or(size_fallback)
        .filter(|size| *size > 0);
    if let Some(size) = size {
        push_one(&mut desc, "SIZE", &size.to_string());
    }

    let reason = match reason {
        InstallReason::Explicit => "0",
        InstallReason::Dep => "1",
    };
    push_one(&mut desc, "REASON", reason);
    push_one(&mut desc, "VALIDATION", "none");

    if let Some(meta) = meta {
        push_section(&mut desc, "LICENSE", &meta.license);
    }
    push_section(&mut desc, "DEPENDS", depends);
    if let Some(meta) = meta {
        push_section(&mut desc, "OPTDEPENDS", &meta.optdepends);
    }
    push_section(&mut desc, "PROVIDES", provides);
    if let Some(meta) = meta {
        push_section(&mut desc, "CONFLICTS", &meta.conflicts);
        push_section(&mut desc, "REPLACES", &meta.replaces);
        if !meta.groups.is_empty() {
            push_section(&mut desc, "GROUPS", &meta.groups);
        } else if let Some(groups) = meta.extra.get("group") {
            push_section(&mut desc, "GROUPS", groups);
        }
    }

    desc
}

fn push_one(out: &mut String, key: &str, value: &str) {
    if value.trim().is_empty() {
        return;
    }
    out.push('%');
    out.push_str(key);
    out.push_str("%\n");
    out.push_str(value.trim());
    out.push('\n');
    out.push('\n');
}

fn push_section(out: &mut String, key: &str, values: &[String]) {
    let values: Vec<&str> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect();
    if values.is_empty() {
        return;
    }

    out.push('%');
    out.push_str(key);
    out.push_str("%\n");
    for value in values {
        out.push_str(value);
        out.push('\n');
    }
    out.push('\n');
}

fn extra_first(meta: &PackageMeta, key: &str) -> Option<String> {
    meta.extra.get(key).and_then(|values| {
        values
            .iter()
            .find(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
    })
}

fn collect_files(xpm_local: &Path, name: &str) -> XpmResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    for raw in local_db::read_files(xpm_local, name)? {
        if raw.starts_with("@ABS:") {
            continue;
        }
        let Some(path) = normalize_pacman_path(&raw) else {
            continue;
        };
        if path.starts_with('.') {
            continue;
        }
        if seen.insert(path.clone()) {
            files.push(path);
        }
    }

    Ok(files)
}

fn collect_backup_hashes(
    xpm_local: &Path,
    name: &str,
    meta: Option<&PackageMeta>,
    pkg_file: Option<&Path>,
    root_dir: &Path,
) -> XpmResult<Vec<(String, String)>> {
    let paths = match meta {
        Some(meta) => meta.backup.clone(),
        None => local_db::read_backup_list(xpm_local, name)?,
    };

    let mut seen = HashSet::new();
    let mut backups = Vec::new();

    for raw in paths {
        let Some(path) = normalize_pacman_path(&raw) else {
            return Err(XpmError::Database(format!(
                "invalid backup path '{raw}' for package '{name}'"
            )));
        };
        let path = path.trim_end_matches('/').to_string();
        if path.is_empty() {
            return Err(XpmError::Database(format!(
                "invalid backup path '{raw}' for package '{name}'"
            )));
        }
        if !seen.insert(path.clone()) {
            continue;
        }

        let hash = backup_md5(&path, pkg_file, root_dir)?;
        backups.push((path, hash));
    }

    Ok(backups)
}

fn backup_md5(path: &str, pkg_file: Option<&Path>, root_dir: &Path) -> XpmResult<String> {
    if let Some(pkg_file) = pkg_file {
        if let Some(bytes) = read_raw_entry(pkg_file, path)? {
            return Ok(format!("{:x}", Md5::digest(&bytes)));
        }
    }

    let on_disk = root_dir.join(path);
    match fs::read(&on_disk) {
        Ok(bytes) => Ok(format!("{:x}", Md5::digest(&bytes))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(XpmError::Database(format!(
            "cannot hash backup file '{path}': not in package archive and missing at {}",
            on_disk.display()
        ))),
        Err(err) => Err(XpmError::Database(format!(
            "cannot read backup file {}: {err}",
            on_disk.display()
        ))),
    }
}

fn render_files(files: &[String], backups: &[(String, String)]) -> String {
    let mut out = String::from(local_db::FILES_HEADER);
    out.push('\n');
    for path in files {
        out.push_str(path);
        out.push('\n');
    }
    out.push('\n');

    if !backups.is_empty() {
        out.push_str("%BACKUP%\n");
        for (path, hash) in backups {
            out.push_str(path);
            out.push('\t');
            out.push_str(hash);
            out.push('\n');
        }
        out.push('\n');
    }

    out
}

fn normalize_pacman_path(raw: &str) -> Option<String> {
    let cleaned = raw.replace('\\', "/");
    let trailing_slash = cleaned.ends_with('/');
    let mut parts = Vec::new();

    for component in Path::new(&cleaned).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }

    if parts.is_empty() {
        return None;
    }

    let mut path = parts.join("/");
    if trailing_slash {
        path.push('/');
    }
    Some(path)
}

fn resolve_version(
    xpm_local: &Path,
    name: &str,
    version: &str,
    pkg_file: Option<&Path>,
) -> XpmResult<String> {
    let requested = version.trim();
    if !requested.is_empty() {
        return Ok(requested.to_string());
    }

    if let Some(recorded) = local_db::read_version(xpm_local, name) {
        return Ok(recorded);
    }

    if let Some(pkg_file) = pkg_file {
        let meta = read_metadata(pkg_file)?.meta;
        if !meta.version.is_empty() {
            return Ok(meta.full_version());
        }
    }

    Err(XpmError::Database(format!(
        "no version recorded for package '{name}'"
    )))
}

fn install_date() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn pacman_entry_exists(pacman_local: &Path, name: &str, version: &str) -> bool {
    let expected = format!("{name}-{version}");
    fs::read_dir(pacman_local)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            entry.file_name().to_string_lossy() == expected
                && entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
        })
}

fn find_cached_package(cache_dir: &Path, name: &str, version: &str) -> Option<PathBuf> {
    if !cache_dir.is_dir() {
        return None;
    }

    let prefix = format!("{name}-{version}-");
    let mut candidates: Vec<PathBuf> = fs::read_dir(cache_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|file_name| file_name.to_str())
                .is_some_and(|file_name| {
                    file_name.starts_with(&prefix)
                        && (file_name.ends_with(".xp") || file_name.ends_with(".pkg.tar.zst"))
                })
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

fn ensure_db_version(pacman_local: &Path) -> XpmResult<()> {
    let version_path = pacman_local.join(ALPM_DB_VERSION);

    match fs::read_to_string(&version_path) {
        Ok(raw) => validate_db_version(&raw, &version_path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if !local_is_empty(pacman_local)? {
                return Err(XpmError::Database(format!(
                    "pacman local database '{}' is missing {ALPM_DB_VERSION}; run pacman-db-upgrade (xpm will not)",
                    pacman_local.display()
                )));
            }

            create_dir_mode(pacman_local, 0o755)?;
            write_initial_db_version(&version_path)
        }
        Err(err) => Err(err.into()),
    }
}

fn validate_db_version(raw: &str, path: &Path) -> XpmResult<()> {
    let value = raw.trim();
    match value.parse::<u32>() {
        Ok(ALPM_DB_VERSION_SUPPORTED) => Ok(()),
        Ok(other) => Err(XpmError::Database(format!(
            "unsupported {ALPM_DB_VERSION} {other} in '{}' (expected {ALPM_DB_VERSION_SUPPORTED})",
            path.display()
        ))),
        Err(_) => Err(XpmError::Database(format!(
            "invalid {ALPM_DB_VERSION} '{value}' in '{}'",
            path.display()
        ))),
    }
}

fn write_initial_db_version(path: &Path) -> XpmResult<()> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .open(path)
    {
        Ok(mut file) => {
            file.set_permissions(fs::Permissions::from_mode(0o644))?;
            file.write_all(b"9\n")?;
            file.sync_all()?;
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let raw = fs::read_to_string(path)?;
            validate_db_version(&raw, path)
        }
        Err(err) => Err(err.into()),
    }
}

fn local_is_empty(local: &Path) -> XpmResult<bool> {
    match fs::read_dir(local) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err.into()),
    }
}

fn db_path(pacman_local: &Path) -> XpmResult<PathBuf> {
    match pacman_local.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent.to_path_buf()),
        Some(_) => Ok(PathBuf::from(".")),
        None => Err(XpmError::Database(format!(
            "invalid pacman local database path '{}'",
            pacman_local.display()
        ))),
    }
}

fn remove_matching_dirs(pacman_local: &Path, name: &str) -> XpmResult<usize> {
    let entries = match fs::read_dir(pacman_local) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.into()),
    };

    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(dir_name) = file_name.to_str() else {
            continue;
        };
        if package_name_from_entry(dir_name) == Some(name) {
            fs::remove_dir_all(entry.path())?;
            removed += 1;
        }
    }

    Ok(removed)
}

fn package_name_from_entry(dir_name: &str) -> Option<&str> {
    let mut parts = dir_name.rsplitn(3, '-');
    let _release = parts.next()?;
    let _version = parts.next()?;
    let name = parts.next()?;
    (!name.is_empty()).then_some(name)
}

fn validate_package_name(name: &str) -> XpmResult<()> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('-')
        && !name.starts_with('.')
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'.' | b'_' | b'+' | b'-')
        });

    if valid {
        Ok(())
    } else {
        Err(XpmError::Database(format!(
            "invalid package name '{name}' for pacman database entry"
        )))
    }
}

fn validate_version(version: &str) -> XpmResult<()> {
    let invalid = || {
        XpmError::Database(format!(
            "invalid package version '{version}' for pacman database entry"
        ))
    };

    if version.is_empty() || version.contains('/') || version.contains('\\') {
        return Err(invalid());
    }
    if version.chars().any(char::is_whitespace) {
        return Err(invalid());
    }

    let mut parts = version.split('-');
    let Some(pkgver) = parts.next() else {
        return Err(invalid());
    };
    let Some(pkgrel) = parts.next() else {
        return Err(invalid());
    };
    if parts.next().is_some() || pkgver.is_empty() || pkgrel.is_empty() {
        return Err(invalid());
    }

    let version_part = match pkgver.split_once(':') {
        Some((epoch, rest)) => {
            if epoch.is_empty()
                || !epoch.bytes().all(|byte| byte.is_ascii_digit())
                || rest.is_empty()
                || rest.contains(':')
            {
                return Err(invalid());
            }
            rest
        }
        None => pkgver,
    };

    let allowed =
        |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'+' | b'~');
    if !version_part.bytes().all(allowed) || !pkgrel.bytes().all(allowed) {
        return Err(invalid());
    }

    Ok(())
}

fn create_dir_mode(path: &Path, mode: u32) -> XpmResult<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

fn write_file_mode(path: &Path, data: &[u8]) -> XpmResult<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o644)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o644))?;
    file.write_all(data)?;
    file.sync_all()?;
    Ok(())
}

fn sync_dir(path: &Path) {
    if let Ok(dir) = File::open(path) {
        let _ = dir.sync_all();
    }
}

fn cleanup_staging(staging: &Path) {
    let _ = fs::remove_dir_all(staging);
    if let Some(parent) = staging.parent() {
        let _ = fs::remove_dir(parent);
    }
}

struct DbLock {
    path: PathBuf,
}

impl DbLock {
    fn acquire(dbpath: &Path) -> XpmResult<Self> {
        if !dbpath.exists() {
            create_dir_mode(dbpath, 0o755)?;
        }

        let path = dbpath.join(DB_LOCK);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o000)
            .open(&path)
        {
            Ok(_) => Ok(Self { path }),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(XpmError::Database(
                format!("pacman database is locked: {} exists", path.display()),
            )),
            Err(err) => Err(XpmError::Database(format!(
                "failed to acquire pacman database lock {}: {err}",
                path.display()
            ))),
        }
    }
}

impl Drop for DbLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;

    use tempfile::TempDir;

    const CONF: &[u8] = b"port=1234\n";

    fn write_package(dir: &Path, file_name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
        let mut raw = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw);
            for (path, data) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_path(path).expect("tar path");
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_uid(0);
                header.set_gid(0);
                header.set_cksum();
                builder.append(&header, *data).expect("append tar entry");
            }
            builder.finish().expect("finish tar");
        }

        let compressed = zstd::encode_all(&raw[..], 1).expect("compress");
        let path = dir.join(file_name);
        let mut file = File::create(&path).expect("create package");
        file.write_all(&compressed).expect("write package");
        path
    }

    fn sample_package(dir: &Path) -> PathBuf {
        let pkginfo = b"pkgname = hello\n\
            pkgver = 1:2.3.4-5\n\
            pkgdesc = Hello tool\n\
            url = https://example.com/hello\n\
            builddate = 1700000000\n\
            size = 42\n\
            arch = x86_64\n\
            license = MIT\n\
            license = Apache\n\
            depend = glibc\n\
            optdepend = bash: completion\n\
            provides = hello-world=1:2.3.4-5\n\
            conflict = hello-old\n\
            replaces = oldhello\n\
            group = base\n\
            pkgbase = hello-src\n\
            packager = Test Packager <packager@example.com>\n\
            backup = etc/hello.conf\n";
        write_package(
            dir,
            "hello-1:2.3.4-5-x86_64.pkg.tar.zst",
            &[
                (".PKGINFO", pkginfo),
                ("etc/hello.conf", CONF),
                ("usr/bin/hello", b"hello"),
            ],
        )
    }

    fn write_xpm_entry(
        xpm_local: &Path,
        name: &str,
        version: &str,
        reason: InstallReason,
        files: &[&str],
        backups: &[&str],
        mtree: Option<&[u8]>,
        install: Option<&[u8]>,
    ) {
        let dir = xpm_local.join(name);
        fs::create_dir_all(&dir).expect("xpm entry");
        fs::write(dir.join(local_db::VERSION_FILE), version).expect("version");
        fs::write(
            dir.join(crate::install_reason::REASON_FILE),
            reason.as_str(),
        )
        .expect("reason");

        let files: Vec<String> = files.iter().map(|path| path.to_string()).collect();
        local_db::write_file_list(xpm_local, name, &files).expect("files");
        let backups: Vec<String> = backups.iter().map(|path| path.to_string()).collect();
        local_db::write_backup_list(xpm_local, name, &backups).expect("backups");

        if let Some(raw) = mtree {
            local_db::write_mtree(xpm_local, name, raw).expect("mtree");
        }
        if let Some(install) = install {
            fs::write(dir.join("install"), install).expect("install");
        }
    }

    fn write_pacman_version(local: &Path) {
        fs::create_dir_all(local).expect("pacman local");
        fs::write(local.join(ALPM_DB_VERSION), "9\n").expect("db version");
    }

    fn write_pacman_entry(local: &Path, name: &str, version: &str) {
        let dir = local.join(format!("{name}-{version}"));
        fs::create_dir_all(&dir).expect("pacman entry");
        fs::write(dir.join("desc"), "%NAME%\nstale\n\n").expect("desc");
        fs::write(dir.join("files"), "%FILES%\n\n").expect("files");
    }

    fn sections(desc: &str) -> HashMap<String, Vec<String>> {
        crate::repo_db::parse_alpm_sections(desc)
    }

    fn section(sections: &HashMap<String, Vec<String>>, key: &str) -> Vec<String> {
        sections.get(key).cloned().unwrap_or_default()
    }

    fn entry_dirs(local: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(local)
            .expect("read local")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn md5_hex(bytes: &[u8]) -> String {
        format!("{:x}", Md5::digest(bytes))
    }

    #[test]
    fn mirrors_desc_files_mtree_install_and_packaged_backup_hash() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        let xpm_local = tmp.path().join("xpm").join("local");
        let root = tmp.path().join("root");
        let pkg = sample_package(tmp.path());
        let mtree = b"#mtree\n./etc type=dir mode=0755 uid=0 gid=0\n./etc/hello.conf type=file mode=0644 size=10 uid=0 gid=0\n";
        let install = b"post_install() { :; }\n";

        write_xpm_entry(
            &xpm_local,
            "hello",
            "1:2.3.4-5\n",
            InstallReason::Dep,
            &[
                "usr/",
                "usr/bin/",
                "usr/bin/hello",
                "etc/",
                "etc/hello.conf",
                "@ABS:/home/x/.local/bin/hello",
                ".PKGINFO",
            ],
            &["etc/hello.conf"],
            Some(mtree),
            Some(install),
        );

        mirror_entry(
            &pacman_local,
            &xpm_local,
            "hello",
            "1:2.3.4-5",
            InstallReason::Dep,
            Some(&pkg),
            &root,
        )
        .expect("mirror");

        let entry = pacman_local.join("hello-1:2.3.4-5");
        assert!(entry.is_dir());
        assert_eq!(
            entry
                .metadata()
                .expect("entry metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755
        );

        let desc = fs::read_to_string(entry.join("desc")).expect("desc");
        let desc_sections = sections(&desc);
        assert_eq!(section(&desc_sections, "NAME"), vec!["hello"]);
        assert_eq!(section(&desc_sections, "VERSION"), vec!["1:2.3.4-5"]);
        assert_eq!(section(&desc_sections, "BASE"), vec!["hello-src"]);
        assert_eq!(section(&desc_sections, "DESC"), vec!["Hello tool"]);
        assert_eq!(
            section(&desc_sections, "URL"),
            vec!["https://example.com/hello"]
        );
        assert_eq!(section(&desc_sections, "ARCH"), vec!["x86_64"]);
        assert_eq!(section(&desc_sections, "BUILDDATE"), vec!["1700000000"]);
        assert_eq!(section(&desc_sections, "PACKAGER").len(), 1);
        assert_eq!(section(&desc_sections, "SIZE"), vec!["42"]);
        assert_eq!(section(&desc_sections, "REASON"), vec!["1"]);
        assert_eq!(section(&desc_sections, "VALIDATION"), vec!["none"]);
        assert_eq!(section(&desc_sections, "LICENSE"), vec!["MIT", "Apache"]);
        assert_eq!(section(&desc_sections, "DEPENDS"), vec!["glibc"]);
        assert_eq!(
            section(&desc_sections, "OPTDEPENDS"),
            vec!["bash: completion"]
        );
        assert_eq!(
            section(&desc_sections, "PROVIDES"),
            vec!["hello-world=1:2.3.4-5"]
        );
        assert_eq!(section(&desc_sections, "CONFLICTS"), vec!["hello-old"]);
        assert_eq!(section(&desc_sections, "REPLACES"), vec!["oldhello"]);
        assert_eq!(section(&desc_sections, "GROUPS"), vec!["base"]);
        let install_date = section(&desc_sections, "INSTALLDATE");
        assert_eq!(install_date.len(), 1);
        assert!(install_date[0].parse::<u64>().expect("install date") > 0);
        assert!(desc.ends_with("\n\n"));

        let allowed_headers = [
            "%NAME%",
            "%VERSION%",
            "%BASE%",
            "%DESC%",
            "%URL%",
            "%ARCH%",
            "%BUILDDATE%",
            "%INSTALLDATE%",
            "%PACKAGER%",
            "%SIZE%",
            "%REASON%",
            "%VALIDATION%",
            "%LICENSE%",
            "%DEPENDS%",
            "%OPTDEPENDS%",
            "%PROVIDES%",
            "%CONFLICTS%",
            "%REPLACES%",
            "%GROUPS%",
        ];
        for line in desc.lines().filter(|line| line.starts_with('%')) {
            assert!(
                allowed_headers.contains(&line),
                "unexpected desc section {line}"
            );
        }

        let files = fs::read_to_string(entry.join("files")).expect("files");
        let expected = format!(
            "%FILES%\nusr/\nusr/bin/\nusr/bin/hello\netc/\netc/hello.conf\n\n%BACKUP%\netc/hello.conf\t{}\n\n",
            md5_hex(CONF)
        );
        assert_eq!(files, expected);

        assert_eq!(fs::read(entry.join("mtree")).expect("mtree"), mtree);
        assert_eq!(fs::read(entry.join("install")).expect("install"), install);
        for file in ["desc", "files", "mtree", "install"] {
            let mode = entry
                .join(file)
                .metadata()
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o644, "{file} must be 0644");
        }

        assert_eq!(
            fs::read_to_string(pacman_local.join(ALPM_DB_VERSION)).expect("db version"),
            "9\n"
        );
        assert!(!tmp.path().join("pacman").join(DB_LOCK).exists());
    }

    #[test]
    fn reason_maps_explicit_and_dep() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        let xpm_local = tmp.path().join("xpm").join("local");
        let root = tmp.path().join("root");

        write_xpm_entry(
            &xpm_local,
            "alpha",
            "1.0-1",
            InstallReason::Explicit,
            &["usr/bin/alpha"],
            &[],
            None,
            None,
        );
        write_xpm_entry(
            &xpm_local,
            "beta",
            "1.0-1",
            InstallReason::Dep,
            &["usr/bin/beta"],
            &[],
            None,
            None,
        );

        mirror_entry(
            &pacman_local,
            &xpm_local,
            "alpha",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect("explicit");
        mirror_entry(
            &pacman_local,
            &xpm_local,
            "beta",
            "1.0-1",
            InstallReason::Dep,
            None,
            &root,
        )
        .expect("dep");

        let alpha =
            fs::read_to_string(pacman_local.join("alpha-1.0-1").join("desc")).expect("alpha");
        let beta = fs::read_to_string(pacman_local.join("beta-1.0-1").join("desc")).expect("beta");
        assert_eq!(section(&sections(&alpha), "REASON"), vec!["0"]);
        assert_eq!(section(&sections(&beta), "REASON"), vec!["1"]);
    }

    #[test]
    fn upgrade_replaces_old_versions_and_duplicates() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        let xpm_local = tmp.path().join("xpm").join("local");
        let root = tmp.path().join("root");
        write_pacman_version(&pacman_local);
        write_pacman_entry(&pacman_local, "tool", "1.0-1");
        write_pacman_entry(&pacman_local, "tool", "0.9-1");
        write_pacman_entry(&pacman_local, "tool-extra", "9-1");

        write_xpm_entry(
            &xpm_local,
            "tool",
            "2.0-1",
            InstallReason::Explicit,
            &["usr/bin/tool"],
            &[],
            None,
            None,
        );

        mirror_entry(
            &pacman_local,
            &xpm_local,
            "tool",
            "2.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect("upgrade");

        assert_eq!(
            entry_dirs(&pacman_local),
            vec!["tool-2.0-1".to_string(), "tool-extra-9-1".to_string()]
        );

        fs::write(pacman_local.join("tool-2.0-1").join("stale"), b"old").expect("stale marker");
        mirror_entry(
            &pacman_local,
            &xpm_local,
            "tool",
            "2.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect("reinstall");

        assert_eq!(
            entry_dirs(&pacman_local),
            vec!["tool-2.0-1".to_string(), "tool-extra-9-1".to_string()]
        );
        assert!(!pacman_local.join("tool-2.0-1").join("stale").exists());
    }

    #[test]
    fn remove_entry_matches_parsed_name_not_prefix() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        write_pacman_version(&pacman_local);
        write_pacman_entry(&pacman_local, "my-pkg", "1:2.0-1");
        write_pacman_entry(&pacman_local, "my-pkg", "1.0-1");
        write_pacman_entry(&pacman_local, "my-pkg-extra", "1.0-1");
        write_pacman_entry(&pacman_local, "my-pkgtools", "1.0-1");

        remove_entry(&pacman_local, "my-pkg").expect("remove");

        assert_eq!(
            entry_dirs(&pacman_local),
            vec![
                "my-pkg-extra-1.0-1".to_string(),
                "my-pkgtools-1.0-1".to_string()
            ]
        );
        assert_eq!(
            fs::read_to_string(pacman_local.join(ALPM_DB_VERSION)).expect("db version"),
            "9\n"
        );
        assert!(!tmp.path().join("pacman").join(DB_LOCK).exists());
    }

    #[test]
    fn alpm_db_version_missing_nonempty_and_non_nine_are_errors() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");

        let empty_local = tmp.path().join("empty").join("local");
        let empty_xpm = tmp.path().join("empty-xpm").join("local");
        write_xpm_entry(
            &empty_xpm,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            &["usr/bin/foo"],
            &[],
            None,
            None,
        );
        mirror_entry(
            &empty_local,
            &empty_xpm,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect("empty db is initialized");
        assert_eq!(
            fs::read_to_string(empty_local.join(ALPM_DB_VERSION)).expect("version"),
            "9\n"
        );

        let unversioned_local = tmp.path().join("unversioned").join("local");
        let unversioned_xpm = tmp.path().join("unversioned-xpm").join("local");
        write_pacman_entry(&unversioned_local, "foo", "1.0-1");
        write_xpm_entry(
            &unversioned_xpm,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            &["usr/bin/foo"],
            &[],
            None,
            None,
        );
        let err = mirror_entry(
            &unversioned_local,
            &unversioned_xpm,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect_err("missing version with entries must fail");
        assert!(format!("{err}").contains(ALPM_DB_VERSION));
        let err = remove_entry(&unversioned_local, "foo").expect_err("remove must validate too");
        assert!(format!("{err}").contains(ALPM_DB_VERSION));
        assert!(unversioned_local.join("foo-1.0-1").is_dir());

        let old_local = tmp.path().join("old").join("local");
        let old_xpm = tmp.path().join("old-xpm").join("local");
        fs::create_dir_all(&old_local).expect("old local");
        fs::write(old_local.join(ALPM_DB_VERSION), "8\n").expect("old version");
        write_xpm_entry(
            &old_xpm,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            &["usr/bin/foo"],
            &[],
            None,
            None,
        );
        let err = mirror_entry(
            &old_local,
            &old_xpm,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect_err("version 8 must fail");
        assert!(format!("{err}").contains(ALPM_DB_VERSION));
        assert!(!old_local.join("foo-1.0-1").exists());
    }

    #[test]
    fn existing_db_lock_blocks_mutations_without_removing_it() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        let xpm_local = tmp.path().join("xpm").join("local");
        let root = tmp.path().join("root");
        write_pacman_version(&pacman_local);
        let lock = tmp.path().join("pacman").join(DB_LOCK);
        fs::write(&lock, b"").expect("lock");

        write_xpm_entry(
            &xpm_local,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            &["usr/bin/foo"],
            &[],
            None,
            None,
        );

        let err = mirror_entry(
            &pacman_local,
            &xpm_local,
            "foo",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect_err("lock must block mirror");
        assert!(format!("{err}").contains("locked"));
        assert!(!pacman_local.join("foo-1.0-1").exists());
        assert!(lock.exists());

        let err = remove_entry(&pacman_local, "foo").expect_err("lock must block remove");
        assert!(format!("{err}").contains("locked"));
        assert!(lock.exists());
    }

    #[test]
    fn backup_hash_prefers_packaged_bytes_falls_back_and_errors_when_missing() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        let xpm_local = tmp.path().join("xpm").join("local");
        let root = tmp.path().join("root");
        let packaged = b"from-package\n";
        let live = b"from-root\n";
        fs::create_dir_all(root.join("etc")).expect("root etc");
        fs::write(root.join("etc/cache.conf"), live).expect("root file");

        let pkginfo = b"pkgname = cache\npkgver = 1.0-1\nbackup = etc/cache.conf\n";
        let pkg = write_package(
            tmp.path(),
            "cache-1.0-1-x86_64.pkg.tar.zst",
            &[(".PKGINFO", pkginfo), ("etc/cache.conf", packaged)],
        );
        write_xpm_entry(
            &xpm_local,
            "cache",
            "1.0-1",
            InstallReason::Explicit,
            &["etc/cache.conf"],
            &["etc/cache.conf"],
            None,
            None,
        );
        mirror_entry(
            &pacman_local,
            &xpm_local,
            "cache",
            "1.0-1",
            InstallReason::Explicit,
            Some(&pkg),
            &root,
        )
        .expect("mirror with package");
        let files =
            fs::read_to_string(pacman_local.join("cache-1.0-1").join("files")).expect("files");
        assert!(files.contains(&format!("etc/cache.conf\t{}\n", md5_hex(packaged))));
        assert!(!files.contains(&md5_hex(live)));

        let fallback_root = tmp.path().join("fallback-root");
        fs::create_dir_all(fallback_root.join("etc")).expect("fallback etc");
        fs::write(fallback_root.join("etc/cache.conf"), live).expect("fallback file");
        mirror_entry(
            &pacman_local,
            &xpm_local,
            "cache",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &fallback_root,
        )
        .expect("fallback hash");
        let files =
            fs::read_to_string(pacman_local.join("cache-1.0-1").join("files")).expect("files");
        assert!(files.contains(&format!("etc/cache.conf\t{}\n", md5_hex(live))));

        let err = mirror_entry(
            &pacman_local,
            &xpm_local,
            "cache",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &tmp.path().join("missing-root"),
        )
        .expect_err("missing fallback must fail");
        assert!(format!("{err}").contains("backup"));
    }

    #[test]
    fn sync_all_uses_cached_package_and_reports_unrecoverable_packages() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        let xpm_local = tmp.path().join("xpm").join("local");
        let cache = tmp.path().join("cache");
        let root = tmp.path().join("root");
        fs::create_dir_all(&cache).expect("cache");
        fs::create_dir_all(root.join("etc")).expect("root etc");
        fs::write(root.join("etc/cached.conf"), b"modified-live\n").expect("live config");

        let packaged = b"packaged-config\n";
        let pkginfo = b"pkgname = cached\npkgver = 1.0-1\nbackup = etc/cached.conf\n";
        write_package(
            &cache,
            "cached-1.0-1-x86_64.pkg.tar.zst",
            &[(".PKGINFO", pkginfo), ("etc/cached.conf", packaged)],
        );
        write_xpm_entry(
            &xpm_local,
            "cached",
            "1.0-1",
            InstallReason::Explicit,
            &["etc/cached.conf"],
            &["etc/cached.conf"],
            None,
            None,
        );

        let mtree = b"#mtree\n./usr/bin/minimal type=file mode=0755 size=12 uid=0 gid=0\n";
        write_xpm_entry(
            &xpm_local,
            "minimal",
            "2.0-1",
            InstallReason::Dep,
            &["usr/bin/minimal"],
            &[],
            Some(mtree),
            None,
        );

        write_xpm_entry(
            &xpm_local,
            "unsafe",
            "3.0-1",
            InstallReason::Explicit,
            &["etc/unsafe.conf"],
            &["etc/unsafe.conf"],
            None,
            None,
        );

        let report = sync_all(&pacman_local, &xpm_local, &cache, &root).expect("sync");

        assert_eq!(report.written, 2);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "unsafe");
        assert!(report.failed[0].1.contains("no cached package"));

        let cached =
            fs::read_to_string(pacman_local.join("cached-1.0-1").join("files")).expect("files");
        assert!(cached.contains(&format!("etc/cached.conf\t{}\n", md5_hex(packaged))));

        let minimal_desc = fs::read_to_string(pacman_local.join("minimal-2.0-1").join("desc"))
            .expect("minimal desc");
        let minimal_sections = sections(&minimal_desc);
        assert_eq!(section(&minimal_sections, "SIZE"), vec!["12"]);
        assert_eq!(section(&minimal_sections, "REASON"), vec!["1"]);
        assert_eq!(
            fs::read(pacman_local.join("minimal-2.0-1").join("mtree")).expect("mtree"),
            mtree
        );

        assert!(!pacman_local.join("unsafe-3.0-1").exists());
    }

    #[test]
    fn rejects_invalid_names_and_versions() {
        let tmp = TempDir::new().expect("tmp");
        let pacman_local = tmp.path().join("pacman").join("local");
        let xpm_local = tmp.path().join("xpm").join("local");
        let root = tmp.path().join("root");

        assert!(validate_package_name("bad/name").is_err());
        assert!(validate_package_name(".hidden").is_err());
        assert!(validate_package_name("-bad").is_err());
        assert!(validate_package_name("good-name_1.2+@").is_ok());
        assert!(validate_version("1.0").is_err());
        assert!(validate_version("1-2-3").is_err());
        assert!(validate_version("1:2.0-1").is_ok());
        assert!(validate_version("2.0-1.1").is_ok());

        let err = mirror_entry(
            &pacman_local,
            &xpm_local,
            "bad/name",
            "1.0-1",
            InstallReason::Explicit,
            None,
            &root,
        )
        .expect_err("invalid name");
        assert!(format!("{err}").contains("invalid package name"));
    }
}
