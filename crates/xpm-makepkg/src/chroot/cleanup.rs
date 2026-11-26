//! Work-directory cleanup guards and stale-root removal.

use super::*;

// ── Cleanup ─────────────────────────────────────────────────────────────────

pub(super) struct WorkGuard {
    work: PathBuf,
    kind: CopyKind,
    armed: bool,
}

impl WorkGuard {
    pub(super) fn new(work: PathBuf, kind: CopyKind) -> Self {
        Self {
            work,
            kind,
            armed: true,
        }
    }
}

impl Drop for WorkGuard {
    fn drop(&mut self) {
        if self.armed {
            cleanup_work(&self.work, self.kind);
        }
    }
}

pub(super) fn cleanup_work(work: &Path, kind: CopyKind) {
    tracing::debug!(kind = kind.as_str(), work = %work.display(), "cleaning chroot work directory");
    let root = work.join("root");
    let _ = root_command("umount").args(["-R"]).arg(&root).status();
    // Deleting a btrfs subvolume with `rm -rf` leaves an empty subvolume
    // behind. Detect it instead of relying on the marker: a killed build may
    // not have written the marker yet.
    if is_btrfs_subvolume(&root) {
        let _ = root_command("btrfs")
            .args(["subvolume", "delete"])
            .arg(&root)
            .status();
    }
    let _ = root_command("rm").args(["-rf"]).arg(work).status();
}

pub(super) fn cleanup_stale_works(template_dir: &Path) {
    let entries = match fs::read_dir(template_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("work-") {
            continue;
        }
        let work = entry.path();
        if !work.is_dir() {
            continue;
        }
        let lock_path = work.join(".lock");
        let Ok(lock) = File::open(&lock_path) else {
            continue;
        };
        let locked = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if locked != 0 {
            continue;
        }
        let kind = fs::read_to_string(work.join(".copy-kind"))
            .ok()
            .and_then(|value| CopyKind::parse(value.trim()))
            .unwrap_or(CopyKind::Copy);
        cleanup_work(&work, kind);
    }
}

pub(super) fn remove_tree(path: &Path) {
    if !path.exists() {
        return;
    }
    if is_btrfs_subvolume(path) {
        let _ = root_command("btrfs")
            .args(["subvolume", "delete"])
            .arg(path)
            .status();
    }
    let _ = root_command("rm").args(["-rf"]).arg(path).status();
}
