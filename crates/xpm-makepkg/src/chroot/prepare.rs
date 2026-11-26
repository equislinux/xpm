//! Disposable root preparation: users, helpers and host keyring.

use super::*;

// ── Disposable root preparation ─────────────────────────────────────────────

pub(super) fn prepare_work(
    work: &Path,
    source_dir: &Path,
    options: &BuildOptions,
    build_uid: u32,
) -> BuildResult<()> {
    let root = work.join("root");
    ensure_chroot_mountpoints(&root)?;
    if !is_mountpoint(&root) {
        root_command("mount")
            .args(["--bind"])
            .arg(&root)
            .arg(&root)
            .status()
            .map_err(|err| BuildError::Command(format!("bind chroot root: {err}")))?;
    }

    for dir in ["build", "pkgdest", "srcdest", "builddir", "preinstall"] {
        run_root("mkdir", [OsStr::new("-p"), root.join(dir).as_os_str()])?;
    }
    for dir in ["build", "pkgdest", "srcdest", "builddir"] {
        let path = root.join(dir);
        let _ = root_command("chown")
            .arg(format!("{build_uid}:{build_uid}"))
            .arg(&path)
            .status();
    }

    let source_with_dot = format!("{}/.", source_dir.display());
    run_root(
        "cp",
        [
            OsStr::new("-a"),
            OsStr::new("--reflink=auto"),
            OsStr::new(&source_with_dot),
            root.join("build").as_os_str(),
        ],
    )?;
    let _ = root_command("chown")
        .args(["-R", &format!("{build_uid}:{build_uid}")])
        .arg(root.join("build"))
        .status();

    setup_build_user(&root, build_uid)?;
    install_helpers(&root)?;
    mount_shared_paths(&root, options, build_uid)?;

    if !options.chroot_install.is_empty() {
        let mut command = root_command("arch-chroot");
        command
            .arg(&root)
            .arg("/usr/local/bin/xpm")
            .args([
                "--no-confirm",
                "--no-pacman-mirror",
                "install",
                "--as-deps",
                "--needed",
                "--",
            ])
            .args(&options.chroot_install);
        let status = command
            .status()
            .map_err(|err| BuildError::Command(format!("install chroot deps: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(
                "could not install build dependencies into the chroot".into(),
            ));
        }
    }

    if !options.preinstall.is_empty() {
        let preinstall = root.join("preinstall");
        for package in &options.preinstall {
            let name = package.file_name().ok_or_else(|| {
                BuildError::Command(format!("invalid preinstall path {}", package.display()))
            })?;
            run_root(
                "cp",
                [package.as_os_str(), preinstall.join(name).as_os_str()],
            )?;
        }
        let mut command = root_command("arch-chroot");
        command.arg(&root).arg("/usr/local/bin/xpm").args([
            "--no-confirm",
            "--no-pacman-mirror",
            "install",
            "--as-deps",
            "--",
        ]);
        for package in &options.preinstall {
            let name = package.file_name().ok_or_else(|| {
                BuildError::Command(format!("invalid preinstall path {}", package.display()))
            })?;
            command.arg(format!("/preinstall/{}", name.to_string_lossy()));
        }
        let status = command
            .status()
            .map_err(|err| BuildError::Command(format!("preinstall AUR deps: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(
                "could not preinstall AUR dependencies into the chroot".into(),
            ));
        }
    }

    // Package installs may replace /usr/bin/pacman with the real package.
    // Reinstall the shims immediately before makepkg runs.
    install_helpers(&root)?;
    Ok(())
}

pub(super) fn setup_build_user(root: &Path, build_uid: u32) -> BuildResult<()> {
    let script = format!(
        "set -e; \
         if ! id -u build >/dev/null 2>&1; then \
             useradd -m -u {build_uid} -s /bin/bash build; \
         else \
             usermod -o -u {build_uid} build; \
         fi; \
         mkdir -p /home/build; chown {build_uid}:{build_uid} /home/build; \
         printf '%s ALL=(ALL) NOPASSWD: ALL\\n' build > /etc/sudoers.d/build; \
         chmod 440 /etc/sudoers.d/build"
    );
    let status = root_command("arch-chroot")
        .arg(root)
        .args(["bash", "-lc", &script])
        .status()
        .map_err(|err| BuildError::Command(format!("chroot user setup: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "chroot user setup exited with status {:?}",
            status.code()
        )));
    }
    Ok(())
}

pub(super) fn install_helpers(root: &Path) -> BuildResult<()> {
    let xpm = current_xpm()?;
    let arch_path = root.join("etc/xpm-arch");
    run_root(
        "sh",
        [
            OsStr::new("-c"),
            OsStr::new("printf '%s\\n' \"$1\" > \"$2\""),
            OsStr::new("sh"),
            OsStr::new(&current_arch()),
            arch_path.as_os_str(),
        ],
    )?;

    let shim_source = std::env::temp_dir().join(format!(
        "xpm-pacman-shim-{}-{}.sh",
        std::process::id(),
        now_epoch_nanos()
    ));
    fs::write(&shim_source, include_str!("../pacman-shim.sh")).map_err(|source| {
        BuildError::Io {
            path: shim_source.display().to_string(),
            source,
        }
    })?;

    let pacman = root.join("usr/bin/pacman");
    if pacman.is_file() {
        let real = root.join("usr/bin/pacman.real");
        if !real.exists() {
            let _ = root_command("cp").arg(&pacman).arg(&real).status();
        }
    }
    for target in [
        root.join("usr/local/bin/pacman"),
        root.join("usr/bin/pacman"),
        root.join("usr/local/bin/xpm"),
    ] {
        let source = if target.ends_with("xpm") {
            &xpm
        } else {
            &shim_source
        };
        let status = root_command("install")
            .args(["-Dm755"])
            .arg(source)
            .arg(&target)
            .status()
            .map_err(|err| BuildError::Command(format!("install helper: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(format!(
                "could not install {}",
                target.display()
            )));
        }
    }
    let _ = fs::remove_file(&shim_source);
    Ok(())
}

pub(super) fn copy_host_keyring(root: &Path) -> BuildResult<()> {
    let host = Path::new("/etc/xpm/gnupg");
    if !host.is_dir() {
        return Ok(());
    }
    let target = root.join("etc/xpm/gnupg");
    run_root("mkdir", [OsStr::new("-p"), target.as_os_str()])?;
    for name in [
        "pubring.gpg",
        "pubring.kbx",
        "trustdb.gpg",
        "gpg.conf",
        "common.conf",
    ] {
        let source = host.join(name);
        let readable = root_command("test")
            .args(["-f"])
            .arg(&source)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if readable {
            let status = root_command("cp")
                .arg(&source)
                .arg(target.join(name))
                .status()
                .map_err(|err| BuildError::Command(format!("copy keyring: {err}")))?;
            if !status.success() {
                return Err(BuildError::Command(format!(
                    "could not copy keyring file {}",
                    source.display()
                )));
            }
        }
    }
    Ok(())
}
