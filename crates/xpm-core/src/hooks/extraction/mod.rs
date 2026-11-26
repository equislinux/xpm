//! Package file extraction hook and backup/config handling.

use super::*;

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
        // Use the metadata parsed by the install loop when available: this
        // avoids decompressing the package several times per transaction.
        let cached = context.metadata.as_ref();
        let (meta, new_mtree, new_mtree_entries) = match cached {
            Some(metadata) => (
                metadata.meta.clone(),
                metadata.mtree_raw.clone(),
                metadata.mtree.clone(),
            ),
            None => {
                let metadata = crate::package::read_metadata(pkg_file)?;
                (metadata.meta, metadata.mtree_raw, metadata.mtree)
            }
        };
        let backup_paths: HashSet<String> = meta.backup.into_iter().collect();
        let new_hashes = if let Some(metadata) = cached {
            crate::local_db::mtree_hash_map(&metadata.mtree)
        } else {
            mtree_hashes(new_mtree.as_deref())
        };

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
        let mut deferred_hardlinks: Vec<(PathBuf, PathBuf, EntryAttrs)> = Vec::new();

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_path_buf();
            let normalized = normalize_archive_path(&path);

            if normalized.is_empty() || is_metadata_path(&normalized) {
                continue;
            }

            let entry_type = entry.header().entry_type();
            let attrs = read_entry_attrs(&mut entry)?;

            if entry_type.is_dir() {
                let dir_path = context.root_dir.join(normalized.trim_end_matches('/'));
                ensure_no_symlink_ancestors(&context.root_dir, &dir_path)?;
                if let Ok(metadata) = fs::symlink_metadata(&dir_path) {
                    if metadata.file_type().is_symlink() {
                        return Err(XpmError::Package(format!(
                            "refusing to replace symlink {} with a directory",
                            dir_path.display()
                        )));
                    }
                }
                fs::create_dir_all(&dir_path)?;
                apply_entry_attrs(&dir_path, &attrs, false)?;
                let mut manifest_path = normalized.clone();
                if !manifest_path.ends_with('/') {
                    manifest_path.push('/');
                }
                installed_files.push(manifest_path);
                continue;
            }

            let is_regular_file = entry_type.is_file();
            let is_executable = attrs.mode & 0o111 != 0;
            let target = context.root_dir.join(&normalized);

            ensure_no_symlink_ancestors(&context.root_dir, &target)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }

            if entry_type.is_hard_link() {
                let link = entry.link_name()?.ok_or_else(|| {
                    XpmError::Package(format!("hardlink entry {} has no link target", normalized))
                })?;
                let link_normalized = normalize_archive_path(&link);
                let link_target = context.root_dir.join(&link_normalized);
                ensure_no_symlink_ancestors(&context.root_dir, &link_target)?;
                deferred_hardlinks.push((target, link_target, attrs));
                continue;
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
                apply_entry_attrs(&pacnew, &attrs, entry_type.is_symlink())?;
                pacnew_files.push(normalized.clone());
                continue;
            }

            if let Ok(metadata) = fs::symlink_metadata(&target) {
                if metadata.file_type().is_symlink() {
                    fs::remove_file(&target)?;
                } else if metadata.is_dir() && !entry_type.is_dir() {
                    return Err(XpmError::Package(format!(
                        "not overwriting directory with file: {}",
                        target.display()
                    )));
                }
            }

            entry.unpack(&target)?;
            apply_entry_attrs(&target, &attrs, entry_type.is_symlink())?;
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

        for (target, source, attrs) in deferred_hardlinks {
            // A package may ship a hardlink whose path already exists (for
            // example a Qt tool re-hardlinking an installed binary). Pacman
            // replaces the existing entry; fs::hard_link alone would fail
            // with EEXIST. Remove only a non-directory entry.
            match fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_dir() => {
                    return Err(XpmError::Package(format!(
                        "refusing to replace directory {} with a hardlink",
                        target.display()
                    )));
                }
                Ok(_) => {
                    fs::remove_file(&target)?;
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
            fs::hard_link(&source, &target).map_err(|err| {
                XpmError::Package(format!(
                    "failed to create hardlink {} -> {}: {err}",
                    target.display(),
                    source.display()
                ))
            })?;
            apply_entry_attrs(&target, &attrs, false)?;
            if let Ok(relative) = target.strip_prefix(&context.root_dir) {
                installed_files.push(relative.to_string_lossy().into_owned());
            }
        }

        if context.shell_integration {
            ensure_shell_path_on_bash_zsh()?;
        }

        // ── Persist the new package state in the local database ─────────────
        // Prefer the .MTREE manifest (files, directories and symlinks) in
        // pacman's `files` format; fall back to the extracted entries when the
        // package has no usable manifest.
        let manifest = if new_mtree_entries.is_empty() {
            installed_files.clone()
        } else {
            crate::local_db::mtree_paths(&new_mtree_entries)
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
        let install_script = match cached {
            Some(metadata) => metadata.install_script.clone(),
            None => crate::package::reader::read_raw_entry(pkg_file, ".INSTALL")?,
        };
        if let Some(raw_install) = install_script {
            fs::create_dir_all(&pkg_dir)?;
            fs::write(pkg_dir.join("install"), raw_install)?;
        }

        Ok(())
    }
}

mod backup;
mod mtree;

pub(super) use backup::append_suffix;
use backup::{cleanup_stale_files, should_keep_existing};
pub(super) use mtree::load_installed_hashes;
use mtree::mtree_hashes;
