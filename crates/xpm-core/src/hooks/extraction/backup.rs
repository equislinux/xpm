//! Config-file preservation and stale-file cleanup.

use super::*;

/// Decides whether an existing configuration file must be preserved.
///
/// - Install: overwrite only when the on-disk content is identical to the
///   packaged one; otherwise the existing file wins and gets a `.pacnew`.
/// - Upgrade: overwrite only when the installed hash is known and the file is
///   untouched since install; unknown hashes (legacy installs) are preserved.
pub(super) fn should_keep_existing(
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
pub(super) fn cleanup_stale_files(
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

        // Another target in the same transaction is taking this path over
        // (for example `openssl` -> `openssl-3`). Keep it even when the
        // current package no longer ships it; commit order must not matter.
        if context
            .transaction_paths
            .as_ref()
            .is_some_and(|paths| paths.contains(old_path))
        {
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
pub(crate) fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}
