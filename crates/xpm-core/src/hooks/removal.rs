//! File removal hook and empty-directory cleanup.

use super::*;

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

pub(super) fn remove_tracked_path(target: &Path) -> XpmResult<()> {
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

pub(super) fn prune_empty_ancestors(target: &Path, context: &HookContext) -> XpmResult<()> {
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

pub(super) fn fallback_remove_common_paths(context: &HookContext) -> XpmResult<()> {
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

pub(super) fn candidate_homes() -> Vec<PathBuf> {
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
