//! Cheap disposable root copies: snapshot, reflink, overlay or plain copy.

use super::*;

// ── Cheap disposable copies ─────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CopyKind {
    Snapshot,
    Reflink,
    Overlay,
    Copy,
}

impl CopyKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Reflink => "reflink",
            Self::Overlay => "overlay",
            Self::Copy => "copy",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "snapshot" => Some(Self::Snapshot),
            "reflink" => Some(Self::Reflink),
            "overlay" => Some(Self::Overlay),
            "copy" => Some(Self::Copy),
            _ => None,
        }
    }
}

pub(super) fn clone_template(
    template: &Path,
    work: &Path,
    lock: &TemplateLock,
) -> BuildResult<CopyKind> {
    run_root("mkdir", [OsStr::new("-p"), work.as_os_str()])?;
    let root = work.join("root");
    remove_tree(&root);
    lock.shared()?;

    let kind = if is_btrfs_subvolume(template) && snapshot(template, &root).unwrap_or(false) {
        Some(CopyKind::Snapshot)
    } else {
        None
    };
    let kind = kind
        .or_else(|| reflink(template, &root).then_some(CopyKind::Reflink))
        .or_else(|| overlay(template, work, &root).then_some(CopyKind::Overlay));
    let kind = match kind {
        Some(kind) => kind,
        None => {
            eprintln!(
                ":: reflink/overlay/snapshot unavailable; copying the base-devel template \
                 (this is slower, but the build remains correct)"
            );
            full_copy(template, &root)?;
            CopyKind::Copy
        }
    };

    write_marker(work, ".copy-kind", kind.as_str())?;
    Ok(kind)
}

pub(super) fn is_btrfs_subvolume(path: &Path) -> bool {
    root_command("btrfs")
        .args(["subvolume", "show"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(super) fn snapshot(template: &Path, root: &Path) -> BuildResult<bool> {
    let status = root_command("btrfs")
        .args(["subvolume", "snapshot"])
        .arg(template)
        .arg(root)
        .status()
        .map_err(|err| BuildError::Command(format!("btrfs snapshot: {err}")))?;
    Ok(status.success())
}

pub(super) fn reflink(template: &Path, root: &Path) -> bool {
    remove_tree(root);
    let status = root_command("cp")
        .args(["-a", "--reflink=always"])
        .arg(template)
        .arg(root)
        .status();
    if matches!(status, Ok(status) if status.success()) {
        return true;
    }
    remove_tree(root);
    false
}

pub(super) fn overlay(template: &Path, work: &Path, root: &Path) -> bool {
    let upper = work.join("upper");
    let overlay_work = work.join("overlay-work");
    let prepared = run_root("mkdir", [OsStr::new("-p"), upper.as_os_str()])
        .and_then(|_| run_root("mkdir", [OsStr::new("-p"), overlay_work.as_os_str()]))
        .and_then(|_| run_root("mkdir", [OsStr::new("-p"), root.as_os_str()]));
    if prepared.is_err() {
        return false;
    }
    let options = format!(
        "lowerdir={},upperdir={},workdir={}",
        template.display(),
        upper.display(),
        overlay_work.display()
    );
    let status = root_command("mount")
        .args(["-t", "overlay", "overlay", "-o", &options])
        .arg(root)
        .status();
    if matches!(status, Ok(status) if status.success()) {
        return true;
    }
    let _ = root_command("rm")
        .args(["-rf"])
        .arg(&upper)
        .arg(&overlay_work)
        .arg(root)
        .status();
    false
}

pub(super) fn full_copy(template: &Path, root: &Path) -> BuildResult<()> {
    remove_tree(root);
    run_root(
        "cp",
        [OsStr::new("-a"), template.as_os_str(), root.as_os_str()],
    )
}
