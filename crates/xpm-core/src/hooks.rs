//! Pre/post operation hooks for transactions.
//!
//! Hooks provide extensible points to execute arbitrary logic before and
//! after install, remove, and upgrade operations. Built-in hooks include
//! local database registration and file removal.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar;
use xz2::read::XzDecoder;
use zstd::Decoder;

use crate::error::{XpmError, XpmResult};

/// Extension appended when a modified configuration file must be preserved.
const PACNEW_SUFFIX: &str = ".pacnew";
/// Extension appended when a modified configuration file is left behind by a
/// removal or by an upgrade that drops it (`--nosave` disables this).
const PACSAVE_SUFFIX: &str = ".pacsave";

/// A hook that executes before or after an operation.
pub trait Hook: Send + Sync {
    fn name(&self) -> &str;
    fn run(&self, context: &HookContext) -> XpmResult<()>;
}

/// Context passed to hooks during execution.
#[derive(Clone, Debug)]
pub struct HookContext {
    pub operation_type: OperationType,
    pub pkg_name: String,
    pub pkg_version: String,
    /// Version being replaced (upgrades only). Lets config-file handling find
    /// the previous manifest without guessing.
    pub old_version: Option<String>,
    pub pkg_file: Option<PathBuf>,
    pub root_dir: PathBuf,
    pub local_db_dir: PathBuf,
    /// Keep modified configuration files on remove/upgrade (`--nosave` disables).
    pub save_configs: bool,
    pub shell_integration: bool,
}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum OperationType {
    Install,
    Remove,
    Upgrade,
}

/// Extract package files to the filesystem.
///
/// Configuration files (`.PKGINFO` `backup` entries) are handled like pacman:
/// an existing file that differs from the packaged version is kept and the new
/// version is written next to it as `<file>.pacnew`. On upgrades, files that
/// the new version dropped are removed, and modified configuration files among
/// them are preserved as `<file>.pacsave` (unless `--nosave`).
pub struct FileExtractionHook;

impl Hook for FileExtractionHook {
    fn name(&self) -> &str {
        "file-extraction"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        if context.operation_type == OperationType::Remove {
            return Ok(());
        }

        let pkg_file = context
            .pkg_file
            .as_ref()
            .ok_or_else(|| XpmError::Package("package file not specified".to_string()))?;

        if !pkg_file.exists() {
            return Err(XpmError::Package(format!(
                "package file not found: {}",
                pkg_file.display()
            )));
        }

        // ── Metadata needed for config-file handling ────────────────────────
        // `backup` marks configuration files; `new_hashes` is the integrity
        // map of the incoming package. Upgrades also load the previous
        // manifest, hashes and backup list to detect user modifications.
        let meta = crate::package::read_metadata(pkg_file)?.meta;
        let backup_paths: HashSet<String> = meta.backup.into_iter().collect();

        let new_mtree = crate::package::reader::read_raw_entry(pkg_file, ".MTREE")?;
        let new_hashes = mtree_hashes(new_mtree.as_deref());

        let upgrading = context.operation_type == OperationType::Upgrade;
        let old_manifest = if upgrading {
            crate::local_db::read_files(&context.local_db_dir, &context.pkg_name)?
        } else {
            Vec::new()
        };
        let old_hashes = if upgrading {
            load_installed_hashes(context)?
        } else {
            HashMap::new()
        };
        let old_backup: HashSet<String> = if upgrading {
            crate::local_db::read_backup_list(&context.local_db_dir, &context.pkg_name)?
                .into_iter()
                .collect()
        } else {
            HashSet::new()
        };

        // ── Extract the archive ─────────────────────────────────────────────
        let mut archive = open_package_archive(pkg_file)?;
        let mut installed_files: Vec<String> = Vec::new();
        let mut shell_shims: Vec<PathBuf> = Vec::new();
        let mut pacnew_files: Vec<String> = Vec::new();

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_path_buf();
            let normalized = normalize_archive_path(&path);

            // Skip metadata files and directories.
            if normalized.is_empty() || is_metadata_path(&normalized) || normalized.ends_with('/') {
                continue;
            }

            let is_regular_file = entry.header().entry_type().is_file();
            let mode = entry.header().mode().unwrap_or(0);
            let is_executable = mode & 0o111 != 0;
            let target = context.root_dir.join(&normalized);

            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }

            let write_pacnew = is_regular_file
                && backup_paths.contains(&normalized)
                && should_keep_existing(
                    &target,
                    context.operation_type,
                    old_hashes.get(&normalized),
                    new_hashes.get(&normalized),
                )?;

            if write_pacnew {
                let pacnew = append_suffix(&target, PACNEW_SUFFIX);
                entry.unpack(&pacnew)?;
                pacnew_files.push(normalized.clone());
                continue;
            }

            entry.unpack(&target)?;
            installed_files.push(normalized.clone());

            // For non-root installations, create shims in ~/.local/bin for
            // executable binaries.
            if context.shell_integration
                && is_regular_file
                && is_executable
                && normalized.starts_with("usr/bin/")
            {
                if let Some(shim) = ensure_shell_shim(&normalized, &target)? {
                    shell_shims.push(shim);
                }
            }
        }

        if context.shell_integration {
            ensure_shell_path_on_bash_zsh()?;
        }

        // ── Persist the new package state in the local database ─────────────
        // Prefer the .MTREE manifest (files, directories and symlinks) in
        // pacman's `files` format; fall back to the extracted entries when the
        // package has no usable manifest.
        let manifest = match &new_mtree {
            Some(data) => match crate::package::mtree::parse_mtree(data) {
                Ok(entries) if !entries.is_empty() => crate::local_db::mtree_paths(&entries),
                Ok(_) => installed_files.clone(),
                Err(e) => {
                    tracing::warn!(
                        package = %context.pkg_name,
                        error = %e,
                        "unreadable .MTREE; falling back to the extracted file list"
                    );
                    installed_files.clone()
                }
            },
            None => installed_files.clone(),
        };

        let mut manifest = manifest;
        for shim in shell_shims {
            manifest.push(format!("@ABS:{}", shim.display()));
        }

        if !manifest.is_empty() {
            crate::local_db::write_file_list(&context.local_db_dir, &context.pkg_name, &manifest)?;
        }

        // Configuration-file list for future upgrades/removals.
        let pkg_dir = context.local_db_dir.join(&context.pkg_name);
        if backup_paths.is_empty() {
            let _ = fs::remove_file(pkg_dir.join(crate::local_db::BACKUP_FILE));
        } else {
            let mut backups: Vec<String> = backup_paths.into_iter().collect();
            backups.sort();
            crate::local_db::write_backup_list(&context.local_db_dir, &context.pkg_name, &backups)?;
        }

        // Raw .MTREE keeps the integrity map for the next upgrade (stored
        // byte-for-byte: makepkg ships it gzip-compressed).
        if let Some(data) = &new_mtree {
            crate::local_db::write_mtree(&context.local_db_dir, &context.pkg_name, data)?;
        }

        // Upgrade: remove files the new version no longer ships, preserving
        // modified configuration files as .pacsave.
        if upgrading {
            cleanup_stale_files(context, &old_manifest, &manifest, &old_hashes, &old_backup)?;
        }

        for file in &pacnew_files {
            tracing::warn!(
                package = %context.pkg_name,
                file = %file,
                "installed as {}{} (existing file kept)",
                file,
                PACNEW_SUFFIX
            );
        }

        // Persist .INSTALL scriptlet in local db for future lifecycle hooks.
        if let Some(raw_install) = crate::package::reader::read_raw_entry(pkg_file, ".INSTALL")? {
            fs::create_dir_all(&pkg_dir)?;
            fs::write(pkg_dir.join("install"), raw_install)?;
        }

        Ok(())
    }
}

// ── Extraction helpers ──────────────────────────────────────────────────────

/// Opens a package archive, detecting zstd/gzip/xz/plain-tar by magic bytes.
fn open_package_archive(pkg_file: &Path) -> XpmResult<tar::Archive<Box<dyn Read>>> {
    // Sniff the magic bytes on a throwaway handle so the decoder sees the
    // stream from the beginning.
    let mut magic = [0u8; 6];
    let n = {
        let mut probe = std::io::BufReader::new(fs::File::open(pkg_file)?);
        probe.read(&mut magic)?
    };

    let reader = std::io::BufReader::new(fs::File::open(pkg_file)?);
    let reader: Box<dyn Read> = if n >= 4 && magic[..4] == [0x28, 0xB5, 0x2F, 0xFD] {
        Box::new(
            Decoder::new(reader)
                .map_err(|e| XpmError::Package(format!("failed to decode zstd: {e}")))?,
        )
    } else if n >= 2 && magic[..2] == [0x1F, 0x8B] {
        Box::new(GzDecoder::new(reader))
    } else if n >= 6 && magic[..6] == [0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00] {
        Box::new(XzDecoder::new(reader))
    } else {
        Box::new(reader)
    };

    Ok(tar::Archive::new(reader))
}

/// Normalizes an archive path to the manifest form (`etc/foo.conf`).
/// A trailing `/` (directory entries) is preserved on purpose.
fn normalize_archive_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

/// Whether the entry is package metadata rather than payload.
fn is_metadata_path(normalized: &str) -> bool {
    normalized.ends_with(".PKGINFO")
        || normalized.ends_with(".BUILDINFO")
        || normalized.ends_with(".MTREE")
        || normalized.ends_with(".INSTALL")
}

/// Builds the `path -> sha256` map from raw `.MTREE` bytes.
fn mtree_hashes(mtree: Option<&[u8]>) -> HashMap<String, String> {
    match mtree {
        Some(bytes) => crate::package::mtree::parse_mtree(bytes)
            .map(|entries| crate::local_db::mtree_hash_map(&entries))
            .unwrap_or_default(),
        None => HashMap::new(),
    }
}

/// Loads the installed package's stored `.MTREE` hash map, if any.
fn load_installed_hashes(context: &HookContext) -> XpmResult<HashMap<String, String>> {
    let Some(raw) = crate::local_db::read_mtree(&context.local_db_dir, &context.pkg_name)? else {
        return Ok(HashMap::new());
    };
    Ok(mtree_hashes(Some(raw.as_slice())))
}

/// Decides whether an existing configuration file must be preserved.
///
/// - Install: overwrite only when the on-disk content is identical to the
///   packaged one; otherwise the existing file wins and gets a `.pacnew`.
/// - Upgrade: overwrite only when the installed hash is known and the file is
///   untouched since install; unknown hashes (legacy installs) are preserved.
fn should_keep_existing(
    target: &Path,
    operation: OperationType,
    old_hash: Option<&String>,
    new_hash: Option<&String>,
) -> XpmResult<bool> {
    if target.symlink_metadata().is_err() {
        return Ok(false);
    }

    let current = sha256_file(target)?;
    match operation {
        OperationType::Install => {
            let identical = matches!((new_hash, &current), (Some(new), Some(cur)) if new == cur);
            Ok(!identical)
        }
        OperationType::Upgrade => {
            let unmodified = matches!((old_hash, &current), (Some(old), Some(cur)) if old == cur);
            Ok(!unmodified)
        }
        OperationType::Remove => Ok(false),
    }
}

/// Removes files dropped by an upgrade. Configuration files that were
/// modified are renamed to `.pacsave` first (unless `save_configs` is off).
fn cleanup_stale_files(
    context: &HookContext,
    old_manifest: &[String],
    new_manifest: &[String],
    old_hashes: &HashMap<String, String>,
    old_backup: &HashSet<String>,
) -> XpmResult<()> {
    let new_paths: HashSet<&str> = new_manifest.iter().map(String::as_str).collect();

    for old_path in old_manifest {
        if old_path.is_empty() || old_path.starts_with('%') || old_path.ends_with('/') {
            continue;
        }

        if new_paths.contains(old_path.as_str()) {
            continue;
        }

        // Shell shims live outside the root and are only symlinks.
        if let Some(abs) = old_path.strip_prefix("@ABS:") {
            let _ = fs::remove_file(abs);
            continue;
        }

        let target = context.root_dir.join(old_path);
        if target.symlink_metadata().is_err() {
            continue;
        }

        let is_config = old_backup.contains(old_path);
        let modified = match old_hashes.get(old_path) {
            Some(installed) => sha256_file(&target)?.as_deref() != Some(installed.as_str()),
            None => true,
        };

        if is_config && modified && context.save_configs {
            let pacsave = append_suffix(&target, PACSAVE_SUFFIX);
            fs::rename(&target, &pacsave)?;
            tracing::warn!(
                package = %context.pkg_name,
                file = %old_path,
                "saved as {}{} before removal",
                old_path,
                PACSAVE_SUFFIX
            );
        } else {
            remove_tracked_path(&target)?;
        }
        prune_empty_ancestors(&target, context)?;
    }

    Ok(())
}

/// Appends a suffix to a path without touching its existing extension
/// (`/etc/foo.conf` + `.pacnew` -> `/etc/foo.conf.pacnew`).
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// SHA-256 of a regular file; `None` when it does not exist.
fn sha256_file(path: &Path) -> XpmResult<Option<String>> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let digest = hasher.finalize();
    Ok(Some(
        digest
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
    ))
}

/// Execute pre-operation package scriptlets from `.INSTALL` files.
///
/// Runs before file changes: `pre_install` (install), `pre_upgrade` (upgrade), `pre_remove` (remove).
pub struct PreScriptletHook;

impl Hook for PreScriptletHook {
    fn name(&self) -> &str {
        "pre-scriptlet"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        let Some(script_data) = load_install_script(context)? else {
            return Ok(());
        };

        let functions: &[&str] = match context.operation_type {
            OperationType::Install => &["pre_install"],
            OperationType::Upgrade => &["pre_upgrade"],
            OperationType::Remove => &["pre_remove"],
        };

        if functions.is_empty() {
            return Ok(());
        }

        run_scriptlet_functions(context, &script_data, functions)
    }
}

/// Execute post-operation package scriptlets from `.INSTALL` files.
///
/// Runs after file changes: `post_install` (install), `post_upgrade` (upgrade), `post_remove` (remove).
pub struct PostScriptletHook;

impl Hook for PostScriptletHook {
    fn name(&self) -> &str {
        "post-scriptlet"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        let Some(script_data) = load_install_script(context)? else {
            return Ok(());
        };

        let functions: &[&str] = match context.operation_type {
            OperationType::Install => &["post_install"],
            OperationType::Upgrade => &["post_upgrade"],
            OperationType::Remove => &["post_remove"],
        };

        if functions.is_empty() {
            return Ok(());
        }

        run_scriptlet_functions(context, &script_data, functions)
    }
}

fn load_install_script(context: &HookContext) -> XpmResult<Option<Vec<u8>>> {
    if let Some(pkg_file) = &context.pkg_file {
        if let Some(data) = crate::package::reader::read_raw_entry(pkg_file, ".INSTALL")? {
            return Ok(Some(data));
        }
    }

    let local_install = context.local_db_dir.join(&context.pkg_name).join("install");
    if local_install.exists() {
        return Ok(Some(fs::read(local_install)?));
    }

    Ok(None)
}

fn run_scriptlet_functions(
    context: &HookContext,
    script_data: &[u8],
    functions: &[&str],
) -> XpmResult<()> {
    let tmp_script = context
        .local_db_dir
        .join(format!(".{}.install.tmp", context.pkg_name));
    fs::write(&tmp_script, script_data)?;

    let shell = r#"set -euo pipefail
script_path="$1"
shift
source "$script_path"
for fn in "$@"; do
  if declare -F "$fn" >/dev/null 2>&1; then
    "$fn"
  fi
done
"#;

    let status = Command::new("bash")
        .arg("-c")
        .arg(shell)
        .arg("xpm-scriptlet")
        .arg(&tmp_script)
        .args(functions)
        .current_dir(&context.root_dir)
        .env("XPM_ROOT_DIR", &context.root_dir)
        .env("XPM_PKG_NAME", &context.pkg_name)
        .env("XPM_PKG_VERSION", &context.pkg_version)
        .status();

    let _ = fs::remove_file(&tmp_script);

    let status = status.map_err(|e| {
        XpmError::Package(format!(
            "failed to execute .INSTALL script for '{}': {}",
            context.pkg_name, e
        ))
    })?;

    if !status.success() {
        return Err(XpmError::Package(format!(
            ".INSTALL script failed for '{}' with status {}",
            context.pkg_name, status
        )));
    }

    Ok(())
}

fn ensure_shell_shim(relative_path: &str, target: &Path) -> XpmResult<Option<PathBuf>> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return Ok(None),
    };

    let local_bin = home.join(".local/bin");
    fs::create_dir_all(&local_bin)?;

    let Some(bin_name) = Path::new(relative_path).file_name() else {
        return Ok(None);
    };

    let shim_path = local_bin.join(bin_name);
    if shim_path.exists() || shim_path.symlink_metadata().is_ok() {
        let _ = fs::remove_file(&shim_path);
    }

    std::os::unix::fs::symlink(target, &shim_path)?;
    Ok(Some(shim_path))
}

fn ensure_shell_path_on_bash_zsh() -> XpmResult<()> {
    let home = match std::env::var_os("HOME") {
        Some(h) => PathBuf::from(h),
        None => return Ok(()),
    };

    let marker = "# xpm shell integration";
    let export_line = "export PATH=\"$HOME/.local/bin:$PATH\"";

    for rc in [".bashrc", ".zshrc"] {
        let rc_path = home.join(rc);
        if !rc_path.exists() {
            continue;
        }

        let content = fs::read_to_string(&rc_path).unwrap_or_default();
        if content.contains(marker) {
            continue;
        }

        let mut append = String::new();
        if !content.ends_with('\n') {
            append.push('\n');
        }
        append.push_str(marker);
        append.push('\n');
        append.push_str(export_line);
        append.push('\n');

        let mut merged = content;
        merged.push_str(&append);
        fs::write(rc_path, merged)?;
    }

    Ok(())
}

/// Register installed package in local database.
pub struct LocalDbHook;

impl Hook for LocalDbHook {
    fn name(&self) -> &str {
        "local-db"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        match context.operation_type {
            OperationType::Install | OperationType::Upgrade => {
                let pkg_dir = context.local_db_dir.join(&context.pkg_name);
                fs::create_dir_all(&pkg_dir)?;

                // Write version file
                let version_path = pkg_dir.join("version");
                fs::write(version_path, &context.pkg_version)?;

                // Keep the file list generated by extraction if present.
                let files_path = pkg_dir.join("files");
                if !files_path.exists() {
                    fs::write(files_path, "")?;
                }

                Ok(())
            }
            OperationType::Remove => {
                let pkg_dir = context.local_db_dir.join(&context.pkg_name);
                if pkg_dir.exists() {
                    fs::remove_dir_all(pkg_dir)?;
                }
                Ok(())
            }
        }
    }
}

/// Remove package files from filesystem.
pub struct FileRemovalHook;

impl Hook for FileRemovalHook {
    fn name(&self) -> &str {
        "file-removal"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        if context.operation_type != OperationType::Remove {
            return Ok(()); // Only applies to remove
        }

        let pkg_files_path = context.local_db_dir.join(&context.pkg_name).join("files");

        if !pkg_files_path.exists() {
            // Fallback cleanup for older installs without a manifest.
            fallback_remove_common_paths(context)?;
            return Ok(());
        }

        let file_list = fs::read_to_string(&pkg_files_path)?;
        if file_list.trim().is_empty() {
            // Fallback cleanup for older installs with empty manifest.
            fallback_remove_common_paths(context)?;
            return Ok(());
        }

        // Configuration files modified since install are preserved as
        // `<file>.pacsave` (pacman's `-n`/`--nosave` disables this).
        let backups: HashSet<String> =
            crate::local_db::read_backup_list(&context.local_db_dir, &context.pkg_name)?
                .into_iter()
                .collect();
        let installed_hashes = load_installed_hashes(context)?;

        for file_path in file_list.lines() {
            if file_path.is_empty() || file_path.starts_with('%') {
                continue;
            }

            let is_dir_entry = file_path.ends_with('/');
            let is_shim = file_path.starts_with("@ABS:");

            let target = if let Some(abs) = file_path.strip_prefix("@ABS:") {
                PathBuf::from(abs)
            } else {
                context.root_dir.join(file_path)
            };

            if !is_dir_entry && !is_shim && context.save_configs && backups.contains(file_path) {
                let modified = match installed_hashes.get(file_path) {
                    Some(installed) => sha256_file(&target)?.as_deref() != Some(installed.as_str()),
                    // Legacy install without a stored .MTREE: keep it to be safe.
                    None => target.symlink_metadata().is_ok(),
                };

                if modified && target.symlink_metadata().is_ok() {
                    let pacsave = append_suffix(&target, PACSAVE_SUFFIX);
                    fs::rename(&target, &pacsave)?;
                    tracing::warn!(
                        package = %context.pkg_name,
                        file = %file_path,
                        "saved as {}{}",
                        file_path,
                        PACSAVE_SUFFIX
                    );
                    prune_empty_ancestors(&target, context)?;
                    continue;
                }
            }

            remove_tracked_path(&target)?;
            prune_empty_ancestors(&target, context)?;
        }

        Ok(())
    }
}

fn remove_tracked_path(target: &Path) -> XpmResult<()> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    let file_type = metadata.file_type();
    if file_type.is_symlink() || file_type.is_file() {
        fs::remove_file(target)?;
    } else if file_type.is_dir() {
        let _ = fs::remove_dir(target);
    }

    Ok(())
}

fn prune_empty_ancestors(target: &Path, context: &HookContext) -> XpmResult<()> {
    if context.root_dir == Path::new("/") {
        return Ok(());
    }

    if !target.starts_with(&context.root_dir) {
        return Ok(());
    }

    let mut current = target.parent();
    while let Some(dir) = current {
        if dir == context.root_dir {
            break;
        }

        match fs::remove_dir(dir) {
            Ok(()) => {
                current = dir.parent();
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                current = dir.parent();
            }
            Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                break;
            }
            Err(_) => {
                break;
            }
        }
    }

    Ok(())
}

fn fallback_remove_common_paths(context: &HookContext) -> XpmResult<()> {
    // Conservative fallback: remove the common binary path matching package name.
    let bin_path = context.root_dir.join("usr/bin").join(&context.pkg_name);
    if bin_path.exists() || bin_path.symlink_metadata().is_ok() {
        let _ = fs::remove_file(&bin_path);
    }

    // Remove shims for both current user and original sudo user if available.
    for home in candidate_homes() {
        let shim = home.join(".local/bin").join(&context.pkg_name);
        if shim.exists() || shim.symlink_metadata().is_ok() {
            let _ = fs::remove_file(shim);
        }
    }

    Ok(())
}

fn candidate_homes() -> Vec<PathBuf> {
    let mut homes = Vec::new();

    if let Some(h) = std::env::var_os("HOME") {
        homes.push(PathBuf::from(h));
    }

    if let Some(sudo_user) = std::env::var_os("SUDO_USER") {
        let sudo_user = sudo_user.to_string_lossy();
        let sudo_home = PathBuf::from("/home").join(sudo_user.as_ref());
        if sudo_home.exists() {
            homes.push(sudo_home);
        }
    }

    homes.sort();
    homes.dedup();
    homes
}

/// Load package metadata for inspection during hooks.
///
/// **Note:** Requires full package::reader integration.
pub struct MetadataLoadHook;

impl Hook for MetadataLoadHook {
    fn name(&self) -> &str {
        "metadata-load"
    }

    fn run(&self, context: &HookContext) -> XpmResult<()> {
        if let Some(pkg_file) = &context.pkg_file {
            if !pkg_file.exists() {
                return Err(XpmError::Package(format!(
                    "package file not found: {}",
                    pkg_file.display()
                )));
            }
            // Fail early when the archive is unreadable or its .PKGINFO is
            // invalid, before any file is written to the root.
            crate::package::read_metadata(pkg_file).map_err(|e| {
                XpmError::Package(format!(
                    "invalid package metadata in {}: {e}",
                    pkg_file.display()
                ))
            })?;
        }
        Ok(())
    }
}

/// Hook chain executor — runs multiple hooks in sequence.
pub struct HookChain {
    hooks: Vec<Box<dyn Hook>>,
}

impl HookChain {
    pub fn new() -> Self {
        HookChain { hooks: Vec::new() }
    }

    pub fn add_hook(&mut self, hook: Box<dyn Hook>) {
        self.hooks.push(hook);
    }

    pub fn run(&self, context: &HookContext) -> XpmResult<()> {
        for hook in &self.hooks {
            let result = hook.run(context);
            if let Err(e) = result {
                return Err(XpmError::Package(format!(
                    "hook '{}' failed: {}",
                    hook.name(),
                    e
                )));
            }
        }
        Ok(())
    }

    pub fn hooks(&self) -> &[Box<dyn Hook>] {
        &self.hooks
    }
}

impl Default for HookChain {
    fn default() -> Self {
        let mut chain = HookChain::new();
        // Hook order: pre-scriptlet → file extraction → file removal → post-scriptlet → local db
        chain.add_hook(Box::new(MetadataLoadHook));
        chain.add_hook(Box::new(PreScriptletHook));
        chain.add_hook(Box::new(FileExtractionHook));
        chain.add_hook(Box::new(FileRemovalHook));
        chain.add_hook(Box::new(PostScriptletHook));
        chain.add_hook(Box::new(LocalDbHook));
        chain
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_context(op_type: OperationType) -> (TempDir, TempDir, HookContext) {
        let root_tmp = TempDir::new().expect("create root tempdir");
        let db_tmp = TempDir::new().expect("create db tempdir");

        let ctx = HookContext {
            operation_type: op_type,
            pkg_name: "test".to_string(),
            pkg_version: "1.0-1".to_string(),
            old_version: None,
            pkg_file: None,
            root_dir: root_tmp.path().to_path_buf(),
            local_db_dir: db_tmp.path().to_path_buf(),
            save_configs: true,
            shell_integration: false,
        };

        (root_tmp, db_tmp, ctx)
    }

    #[test]
    fn hook_chain_default_has_hooks() {
        let chain = HookChain::default();
        assert!(!chain.hooks().is_empty());
        assert_eq!(chain.hooks().len(), 6); // metadata, pre-scriptlet, extraction, removal, post-scriptlet, localdb
    }

    #[test]
    fn post_scriptlet_hook_runs_post_install() {
        let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
        let hook = PostScriptletHook;
        ctx.pkg_file = None;

        let pkg_dir = db_tmp.path().join(&ctx.pkg_name);
        fs::create_dir_all(&pkg_dir).expect("create pkg dir");
        fs::write(
            pkg_dir.join("install"),
            "post_install() { touch \"$XPM_ROOT_DIR/scriptlet-ok\"; }\n",
        )
        .expect("write install script");

        hook.run(&ctx).expect("run scriptlet hook");
        assert!(
            root.path().join("scriptlet-ok").exists(),
            "post_install should create marker file"
        );
    }

    #[test]
    fn local_db_hook_creates_version_file() {
        let (_root, _db_tmp, ctx) = test_context(OperationType::Install);
        let hook = LocalDbHook;

        hook.run(&ctx).expect("run hook");

        let version_file = ctx.local_db_dir.join(&ctx.pkg_name).join("version");
        assert!(version_file.exists());

        let content = fs::read_to_string(&version_file).expect("read version file");
        assert_eq!(content, "1.0-1");
    }

    #[test]
    fn local_db_hook_remove_deletes_entry() {
        let (_root, _db_tmp, ctx) = test_context(OperationType::Remove);
        let pkg_dir = ctx.local_db_dir.join(&ctx.pkg_name);

        // Create the entry first
        fs::create_dir_all(&pkg_dir).expect("create pkg dir");
        fs::write(pkg_dir.join("version"), "1.0-1").expect("write version");

        // Now remove it
        let hook = LocalDbHook;
        hook.run(&ctx).expect("run hook");

        assert!(!pkg_dir.exists(), "pkg directory should be deleted");
    }

    #[test]
    fn file_removal_hook_skips_if_no_files() {
        let (_root, _db_tmp, ctx) = test_context(OperationType::Remove);
        let hook = FileRemovalHook;

        // Should not error even if no file list exists
        let result = hook.run(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn file_removal_hook_removes_files_and_prunes_empty_dirs() {
        let (root, _db_tmp, ctx) = test_context(OperationType::Remove);
        let hook = FileRemovalHook;

        let installed = root.path().join("usr/bin/xfetch");
        fs::create_dir_all(installed.parent().expect("parent dir")).expect("create parent dirs");
        fs::write(&installed, "binary").expect("write binary file");
        // A package-owned symlink must be removed as well (roadmap #24).
        let link = root.path().join("usr/bin/xfetch-link");
        std::os::unix::fs::symlink("xfetch", &link).expect("create symlink");

        let pkg_dir = ctx.local_db_dir.join(&ctx.pkg_name);
        fs::create_dir_all(&pkg_dir).expect("create package db dir");
        fs::write(
            pkg_dir.join("files"),
            "%FILES%\nusr/\nusr/bin/\nusr/bin/xfetch\nusr/bin/xfetch-link\n",
        )
        .expect("write files manifest");

        hook.run(&ctx).expect("run hook");

        assert!(!installed.exists(), "installed file should be removed");
        assert!(!link.exists(), "package-owned symlink should be removed");
        assert!(
            !root.path().join("usr/bin").exists(),
            "empty bin directory should be pruned"
        );
    }

    #[test]
    fn hook_chain_runs_all_hooks() {
        let (_root, _db_tmp, ctx) = test_context(OperationType::Install);
        let mut chain = HookChain::new();
        chain.add_hook(Box::new(LocalDbHook));

        let result = chain.run(&ctx);
        assert!(result.is_ok());
    }

    // ── Configuration-file handling (.pacnew / .pacsave) ────────────────

    use sha2::{Digest, Sha256};

    fn sha256_hex(data: &[u8]) -> String {
        let digest = Sha256::digest(data);
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Builds a minimal `.xp` with payload files, a matching `.MTREE` and a
    /// `.PKGINFO` carrying the `backup` list. `version` is `1.0-1` style.
    fn write_config_package(
        dir: &Path,
        name: &str,
        version: &str,
        files: &[(&str, &[u8])],
        backup: &[&str],
    ) -> PathBuf {
        let path = dir.join(format!("{name}-{version}-x86_64.xp"));

        let mut pkginfo =
            format!("pkgname = {name}\npkgver = {version}\npkgdesc = test\narch = x86_64\n");
        for entry in backup {
            pkginfo.push_str(&format!("backup = {entry}\n"));
        }

        let mut mtree = String::from("#mtree\n");
        for (file, content) in files {
            mtree.push_str(&format!(
                "./{file} type=file mode=0644 size={} sha256digest={} uid=0 gid=0\n",
                content.len(),
                sha256_hex(content)
            ));
        }

        let mut raw_tar = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut raw_tar);
            for (entry, data) in [
                (".PKGINFO", pkginfo.as_bytes()),
                (".MTREE", mtree.as_bytes()),
            ] {
                let mut header = tar::Header::new_gnu();
                header.set_path(entry).unwrap();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append(&header, data).unwrap();
            }
            for (file, content) in files {
                let mut header = tar::Header::new_gnu();
                header.set_path(file).unwrap();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                builder.append(&header, *content).unwrap();
            }
            builder.finish().unwrap();
        }

        let compressed = zstd::encode_all(&raw_tar[..], 1).expect("compress");
        fs::write(&path, compressed).expect("write package");
        path
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    #[test]
    fn install_keeps_existing_config_and_writes_pacnew() {
        let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
        fs::create_dir_all(root.path().join("etc")).expect("etc dir");
        fs::write(root.path().join("etc/foo.conf"), "user content").expect("existing config");

        let pkg = write_config_package(
            db_tmp.path(),
            "hello",
            "1.0-1",
            &[("etc/foo.conf", b"package content")],
            &["etc/foo.conf"],
        );
        ctx.pkg_name = "hello".to_string();
        ctx.pkg_file = Some(pkg);

        FileExtractionHook.run(&ctx).expect("extract");

        assert_eq!(read(&root.path().join("etc/foo.conf")), "user content");
        assert_eq!(
            read(&root.path().join("etc/foo.conf.pacnew")),
            "package content"
        );
        // The new version is not owned by the package manifest.
        let files = crate::local_db::read_files(db_tmp.path(), "hello").expect("files");
        assert!(!files.iter().any(|f| f.contains("pacnew")));
        // The backup list and .MTREE are recorded for the next transaction.
        assert_eq!(
            crate::local_db::read_backup_list(db_tmp.path(), "hello").expect("backup"),
            vec!["etc/foo.conf".to_string()]
        );
        assert!(crate::local_db::read_mtree(db_tmp.path(), "hello")
            .expect("mtree")
            .is_some());
    }

    #[test]
    fn install_identical_config_overwrites_without_pacnew() {
        let (root, db_tmp, mut ctx) = test_context(OperationType::Install);
        fs::create_dir_all(root.path().join("etc")).expect("etc dir");
        fs::write(root.path().join("etc/foo.conf"), "same").expect("existing config");

        let pkg = write_config_package(
            db_tmp.path(),
            "hello",
            "1.0-1",
            &[("etc/foo.conf", b"same")],
            &["etc/foo.conf"],
        );
        ctx.pkg_file = Some(pkg);

        FileExtractionHook.run(&ctx).expect("extract");

        assert_eq!(read(&root.path().join("etc/foo.conf")), "same");
        assert!(!root.path().join("etc/foo.conf.pacnew").exists());
    }

    /// Installs v1 through the real hook so the local database gets the
    /// manifest, backup list and hash map that the upgrade will compare with.
    fn install_v1(root: &Path, db: &Path, version: &str, files: &[(&str, &[u8])], backup: &[&str]) {
        let pkg = write_config_package(db, "hello", version, files, backup);
        let ctx = HookContext {
            operation_type: OperationType::Install,
            pkg_name: "hello".to_string(),
            pkg_version: version.to_string(),
            old_version: None,
            pkg_file: Some(pkg),
            root_dir: root.to_path_buf(),
            local_db_dir: db.to_path_buf(),
            save_configs: true,
            shell_integration: false,
        };
        FileExtractionHook.run(&ctx).expect("install v1");
    }

    fn upgrade_context(root: &Path, db: &Path, pkg: PathBuf, from: &str, to: &str) -> HookContext {
        HookContext {
            operation_type: OperationType::Upgrade,
            pkg_name: "hello".to_string(),
            pkg_version: to.to_string(),
            old_version: Some(from.to_string()),
            pkg_file: Some(pkg),
            root_dir: root.to_path_buf(),
            local_db_dir: db.to_path_buf(),
            save_configs: true,
            shell_integration: false,
        }
    }

    #[test]
    fn upgrade_replaces_unmodified_config() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db = tmp.path().join("db");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&db).expect("db");

        install_v1(
            &root,
            &db,
            "1.0-1",
            &[("etc/foo.conf", b"v1")],
            &["etc/foo.conf"],
        );
        assert_eq!(read(&root.join("etc/foo.conf")), "v1");

        let pkg = write_config_package(
            &db,
            "hello",
            "2.0-1",
            &[("etc/foo.conf", b"v2")],
            &["etc/foo.conf"],
        );
        let ctx = upgrade_context(&root, &db, pkg, "1.0-1", "2.0-1");
        FileExtractionHook.run(&ctx).expect("upgrade");

        assert_eq!(read(&root.join("etc/foo.conf")), "v2");
        assert!(!root.join("etc/foo.conf.pacnew").exists());
    }

    #[test]
    fn upgrade_keeps_modified_config_and_writes_pacnew() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db = tmp.path().join("db");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&db).expect("db");

        install_v1(
            &root,
            &db,
            "1.0-1",
            &[("etc/foo.conf", b"v1")],
            &["etc/foo.conf"],
        );
        fs::write(root.join("etc/foo.conf"), "user edited").expect("edit config");

        let pkg = write_config_package(
            &db,
            "hello",
            "2.0-1",
            &[("etc/foo.conf", b"v2")],
            &["etc/foo.conf"],
        );
        let ctx = upgrade_context(&root, &db, pkg, "1.0-1", "2.0-1");
        FileExtractionHook.run(&ctx).expect("upgrade");

        assert_eq!(read(&root.join("etc/foo.conf")), "user edited");
        assert_eq!(read(&root.join("etc/foo.conf.pacnew")), "v2");
    }

    #[test]
    fn upgrade_removes_stale_files_and_pacsaves_modified_config() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db = tmp.path().join("db");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&db).expect("db");

        install_v1(
            &root,
            &db,
            "1.0-1",
            &[
                ("etc/old.conf", b"v1 config"),
                ("usr/share/stale.txt", b"stale"),
            ],
            &["etc/old.conf"],
        );
        fs::write(root.join("etc/old.conf"), "user edited").expect("edit config");

        let pkg =
            write_config_package(&db, "hello", "2.0-1", &[("usr/share/new.txt", b"new")], &[]);
        let ctx = upgrade_context(&root, &db, pkg, "1.0-1", "2.0-1");
        FileExtractionHook.run(&ctx).expect("upgrade");

        // Modified config dropped by v2 is preserved as .pacsave.
        assert!(!root.join("etc/old.conf").exists());
        assert_eq!(read(&root.join("etc/old.conf.pacsave")), "user edited");
        // Plain stale file is gone.
        assert!(!root.join("usr/share/stale.txt").exists());
        assert!(root.join("usr/share/new.txt").exists());
    }

    #[test]
    fn remove_keeps_modified_config_as_pacsave() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db = tmp.path().join("db");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&db).expect("db");

        install_v1(
            &root,
            &db,
            "1.0-1",
            &[("etc/foo.conf", b"v1")],
            &["etc/foo.conf"],
        );
        fs::write(root.join("etc/foo.conf"), "user edited").expect("edit config");

        let ctx = HookContext {
            operation_type: OperationType::Remove,
            pkg_name: "hello".to_string(),
            pkg_version: "1.0-1".to_string(),
            old_version: None,
            pkg_file: None,
            root_dir: root.clone(),
            local_db_dir: db.clone(),
            save_configs: true,
            shell_integration: false,
        };
        FileRemovalHook.run(&ctx).expect("remove");

        assert!(!root.join("etc/foo.conf").exists());
        assert_eq!(read(&root.join("etc/foo.conf.pacsave")), "user edited");
    }

    #[test]
    fn remove_nosave_deletes_modified_config() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db = tmp.path().join("db");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&db).expect("db");

        install_v1(
            &root,
            &db,
            "1.0-1",
            &[("etc/foo.conf", b"v1")],
            &["etc/foo.conf"],
        );
        fs::write(root.join("etc/foo.conf"), "user edited").expect("edit config");

        let ctx = HookContext {
            operation_type: OperationType::Remove,
            pkg_name: "hello".to_string(),
            pkg_version: "1.0-1".to_string(),
            old_version: None,
            pkg_file: None,
            root_dir: root.clone(),
            local_db_dir: db.clone(),
            save_configs: false,
            shell_integration: false,
        };
        FileRemovalHook.run(&ctx).expect("remove");

        assert!(!root.join("etc/foo.conf").exists());
        assert!(!root.join("etc/foo.conf.pacsave").exists());
    }

    #[test]
    fn remove_unmodified_config_deletes_it() {
        let tmp = TempDir::new().expect("tmp");
        let root = tmp.path().join("root");
        let db = tmp.path().join("db");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&db).expect("db");

        install_v1(
            &root,
            &db,
            "1.0-1",
            &[("etc/foo.conf", b"v1")],
            &["etc/foo.conf"],
        );

        let ctx = HookContext {
            operation_type: OperationType::Remove,
            pkg_name: "hello".to_string(),
            pkg_version: "1.0-1".to_string(),
            old_version: None,
            pkg_file: None,
            root_dir: root.clone(),
            local_db_dir: db.clone(),
            save_configs: true,
            shell_integration: false,
        };
        FileRemovalHook.run(&ctx).expect("remove");

        assert!(!root.join("etc/foo.conf").exists());
        assert!(!root.join("etc/foo.conf.pacsave").exists());
    }

    #[test]
    fn metadata_load_hook_rejects_corrupt_package() {
        let (_root, db_tmp, mut ctx) = test_context(OperationType::Install);
        let bad = db_tmp.path().join("bad.xp");
        fs::write(&bad, b"not a package").expect("write bad");
        ctx.pkg_file = Some(bad);

        let err = MetadataLoadHook.run(&ctx).expect_err("must fail");
        assert!(err.to_string().contains("invalid package metadata"));
    }
}
