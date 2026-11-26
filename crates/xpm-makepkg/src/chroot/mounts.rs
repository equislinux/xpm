//! Bind mounts and mountpoint helpers.

use super::*;

pub(super) fn mount_shared_paths(
    root: &Path,
    options: &BuildOptions,
    build_uid: u32,
) -> BuildResult<()> {
    // The guest xpm config always uses the default cache/database paths, so
    // bind there even when the host config overrides them.
    if let Some(cache) = &options.cache_dir {
        let cache = cache.clone();
        let _ = run_root("mkdir", [OsStr::new("-p"), cache.as_os_str()]);
        let target = root.join("var/cache/xpm/pkg");
        run_root("mkdir", [OsStr::new("-p"), target.as_os_str()])?;
        bind_mount(&cache, &target, false)?;
    }

    if let Some(db_path) = &options.xpm_db_path {
        let sync = db_path.join("sync");
        if sync.is_dir() {
            let target = root.join("var/lib/xpm/sync");
            run_root("mkdir", [OsStr::new("-p"), target.as_os_str()])?;
            bind_mount(&sync, &target, true)?;
        }
    }

    if let Some(srcdest) = &options.srcdest {
        let srcdest = srcdest.clone();
        let _ = run_root("mkdir", [OsStr::new("-p"), srcdest.as_os_str()]);
        let _ = root_command("chown")
            .arg(format!("{build_uid}:{build_uid}"))
            .arg(&srcdest)
            .status();
        let target = root.join("srcdest");
        bind_mount(&srcdest, &target, false)?;
    }

    if let Some(builddir) = &options.builddir {
        let builddir = builddir.clone();
        let _ = run_root("mkdir", [OsStr::new("-p"), builddir.as_os_str()]);
        let _ = root_command("chown")
            .arg(format!("{build_uid}:{build_uid}"))
            .arg(&builddir)
            .status();
        let target = root.join("builddir");
        bind_mount(&builddir, &target, false)?;
    }

    Ok(())
}

pub(super) fn bind_mount(source: &Path, target: &Path, read_only: bool) -> BuildResult<()> {
    let status = root_command("mount")
        .args(["--bind"])
        .arg(source)
        .arg(target)
        .status()
        .map_err(|err| BuildError::Command(format!("bind {}: {err}", source.display())))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "could not bind-mount {} on {}",
            source.display(),
            target.display()
        )));
    }
    if read_only {
        let remount = root_command("mount")
            .args(["-o", "remount,ro,bind"])
            .arg(target)
            .status();
        if !matches!(remount, Ok(status) if status.success()) {
            tracing::warn!(
                target = %target.display(),
                "could not remount the shared sync database read-only"
            );
        }
    }
    Ok(())
}

// ── makepkg and artifacts ───────────────────────────────────────────────────

pub(super) fn ensure_chroot_mountpoints(root: &Path) -> BuildResult<()> {
    for dir in ["proc", "sys", "dev", "run", "tmp"] {
        run_root("mkdir", [OsStr::new("-p"), root.join(dir).as_os_str()])?;
    }
    Ok(())
}

pub(super) fn is_mountpoint(path: &Path) -> bool {
    Command::new("mountpoint")
        .arg("-q")
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
