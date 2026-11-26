//! Shared base-devel template lifecycle and locking.

use super::*;

pub(super) fn template_dir(options: &BuildOptions) -> PathBuf {
    options
        .chroot_dir
        .join(format!("template-{}", current_arch()))
}

// ── Template lifecycle ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct TemplateStamp {
    pub(super) schema: u32,
    pub(super) arch: String,
    #[serde(default)]
    pub(super) xpm_version: String,
    #[serde(default)]
    pub(super) config_hash: String,
    pub(super) created_at: u64,
    pub(super) refreshed_at: u64,
}

impl TemplateStamp {
    pub(super) fn new(arch: &str, now: u64) -> Self {
        Self {
            schema: STAMP_SCHEMA,
            arch: arch.to_string(),
            xpm_version: xpm_version(),
            config_hash: config_hash(),
            created_at: now,
            refreshed_at: now,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TemplateAction {
    Fresh,
    Refresh,
    Recreate,
}

pub(super) fn ensure_template(
    root: &Path,
    stamp_path: &Path,
    options: &BuildOptions,
    lock: &TemplateLock,
) -> BuildResult<()> {
    if template_action(root, stamp_path, options) == TemplateAction::Fresh {
        return Ok(());
    }

    lock.exclusive()?;
    // Another builder may have refreshed the template while we waited.
    let action = template_action(root, stamp_path, options);
    if action == TemplateAction::Fresh {
        return Ok(());
    }

    let arch = current_arch();
    let now = now_epoch();
    match action {
        TemplateAction::Recreate => {
            create_template(root)?;
            write_stamp(stamp_path, &TemplateStamp::new(&arch, now))?;
        }
        TemplateAction::Refresh => {
            refresh_template(root, options)?;
            let created_at = read_stamp(stamp_path)
                .map(|stamp| stamp.created_at)
                .unwrap_or(now);
            let stamp = TemplateStamp {
                schema: STAMP_SCHEMA,
                arch,
                xpm_version: xpm_version(),
                config_hash: config_hash(),
                created_at,
                refreshed_at: now,
            };
            write_stamp(stamp_path, &stamp)?;
        }
        TemplateAction::Fresh => {}
    }

    Ok(())
}

pub(super) fn template_action(
    root: &Path,
    stamp_path: &Path,
    options: &BuildOptions,
) -> TemplateAction {
    if !template_is_ready(root) {
        return TemplateAction::Recreate;
    }
    let Some(stamp) = read_stamp(stamp_path) else {
        // Ready legacy root without a stamp: refresh it and create the stamp.
        return TemplateAction::Refresh;
    };
    if stamp.schema != STAMP_SCHEMA || stamp.arch != current_arch() {
        return TemplateAction::Recreate;
    }
    if !stamp.config_hash.is_empty() && stamp.config_hash != config_hash() {
        return TemplateAction::Recreate;
    }
    if options.chroot_refresh || stamp.xpm_version != xpm_version() || options.chroot_ttl_days == 0
    {
        return TemplateAction::Refresh;
    }
    let age = now_epoch().saturating_sub(stamp.refreshed_at);
    if age >= options.chroot_ttl_days.saturating_mul(SECONDS_PER_DAY) {
        return TemplateAction::Refresh;
    }
    TemplateAction::Fresh
}

pub(super) fn template_is_ready(root: &Path) -> bool {
    root.join("usr/bin/bash").is_file()
        && root.join("var/lib/xpm/local").is_dir()
        && root.join("etc/xpm.conf").is_file()
}

pub(super) fn create_template(root: &Path) -> BuildResult<()> {
    remove_tree(root);
    let parent = root.parent().ok_or_else(|| {
        BuildError::Command(format!("invalid chroot template path {}", root.display()))
    })?;
    run_root("mkdir", [OsStr::new("-p"), parent.as_os_str()])?;

    // Keep the template as a plain directory. Builds still use reflink or
    // overlayfs (or a copy), and `rm -rf` can always remove the template
    // without btrfs-specific subvolume bookkeeping.
    run_root("mkdir", [OsStr::new("-p"), root.as_os_str()])?;

    let xpm = current_xpm()?;
    eprintln!(
        ":: Preparing shared base-devel template (first run only): {}",
        root.display()
    );
    let status = root_command(&xpm)
        .args(["--no-confirm", "--no-pacman-mirror", "--root"])
        .arg(root)
        .args(["bootstrap", "base-devel"])
        .status()
        .map_err(|err| BuildError::Command(format!("xpm bootstrap: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "xpm bootstrap exited with status {:?}",
            status.code()
        )));
    }
    ensure_chroot_mountpoints(root)?;
    tracing::info!(template = %root.display(), "base-devel template created");
    Ok(())
}

pub(super) fn refresh_template(root: &Path, options: &BuildOptions) -> BuildResult<()> {
    let xpm = current_xpm()?;
    eprintln!(
        ":: Refreshing shared base-devel template: {}",
        root.display()
    );
    let run = |args: &[&str]| -> BuildResult<()> {
        let status = root_command(&xpm)
            .args(["--no-confirm", "--no-pacman-mirror", "--root"])
            .arg(root)
            .arg("--dbpath")
            .arg(root.join("var/lib/xpm"))
            .arg("--cachedir")
            .arg(root.join("var/cache/xpm/pkg"))
            .args(args)
            .status()
            .map_err(|err| BuildError::Command(format!("xpm {}: {err}", args.join(" "))))?;
        if status.success() {
            Ok(())
        } else {
            Err(BuildError::Command(format!(
                "xpm {} exited with status {:?}",
                args.join(" "),
                status.code()
            )))
        }
    };

    // The template carries a public copy of the native xpm keyring. Refresh
    // it so a stale template can still verify current database signatures.
    copy_host_keyring(root)?;
    run(&["sync", "--force"])?;
    // Keep the keyring package files current, but never run its scriptlet:
    // archlinux-keyring's .INSTALL shells out to pacman-key.
    run(&[
        "install",
        "--needed",
        "--scriptlets",
        "skip",
        "--",
        "archlinux-keyring",
    ])?;
    run(&["upgrade"])?;
    // A group can gain members between snapshots; `upgrade` does not install
    // packages that were never present.
    run(&["install", "--needed", "--", "base-devel"])?;
    tracing::info!(template = %root.display(), ttl_days = options.chroot_ttl_days, "base-devel template refreshed");
    Ok(())
}

pub(super) fn read_stamp(path: &Path) -> Option<TemplateStamp> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub(super) fn write_stamp(path: &Path, stamp: &TemplateStamp) -> BuildResult<()> {
    let text = serde_json::to_vec_pretty(stamp)
        .map_err(|err| BuildError::Command(format!("serialize template stamp: {err}")))?;
    let temporary = path.with_extension("json.tmp");
    match fs::write(&temporary, &text) {
        Ok(()) => fs::rename(&temporary, path).map_err(|source| BuildError::Io {
            path: path.display().to_string(),
            source,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            let _ = fs::remove_file(&temporary);
            write_root_file(path, &text)
        }
        Err(source) => Err(BuildError::Io {
            path: temporary.display().to_string(),
            source,
        }),
    }
}

// ── Template lock ───────────────────────────────────────────────────────────

pub(super) struct TemplateLock {
    file: File,
}

impl TemplateLock {
    pub(super) fn exclusive(&self) -> BuildResult<()> {
        self.flock(libc::LOCK_EX)
    }

    pub(super) fn shared(&self) -> BuildResult<()> {
        self.flock(libc::LOCK_SH)
    }

    pub(super) fn unlock(&self) {
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }

    pub(super) fn flock(&self, operation: libc::c_int) -> BuildResult<()> {
        let result = unsafe { libc::flock(self.file.as_raw_fd(), operation) };
        if result == 0 {
            Ok(())
        } else {
            let source = std::io::Error::last_os_error();
            Err(BuildError::Command(format!(
                "could not lock the chroot template: {source}"
            )))
        }
    }
}

pub(super) fn open_template_lock(template_dir: &Path) -> BuildResult<TemplateLock> {
    run_root("mkdir", [OsStr::new("-p"), template_dir.as_os_str()])?;
    let path = template_dir.join(".lock");
    if !path.exists() {
        let _ = root_command("touch").arg(&path).status();
        let _ = root_command("chmod").args(["0644"]).arg(&path).status();
    }
    // flock() works on a read-only descriptor; keeping the lock file readable
    // lets an unprivileged xpm-aur coordinate with root-owned builds.
    let file = OpenOptions::new()
        .read(true)
        .open(&path)
        .or_else(|_| OpenOptions::new().read(true).write(true).open(&path))
        .map_err(|source| BuildError::Io {
            path: path.display().to_string(),
            source,
        })?;
    Ok(TemplateLock { file })
}
