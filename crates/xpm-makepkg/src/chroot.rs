//! Clean-chroot builds.
//!
//! The design separates three things that used to be mixed together:
//!
//! * one immutable `base-devel` template per architecture;
//! * a shared package cache (and the host sync database) bind-mounted into the
//!   disposable root;
//! * one cheap copy per build (btrfs snapshot, reflink, overlayfs or a plain
//!   copy as the last resort).
//!
//! The template is only refreshed under an exclusive lock and never mutated by
//! a build. Build dependencies are installed inside the disposable copy with
//! `xpm`, so `makepkg` can run with `--nodeps` and the host stays clean.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{
    collect_packages, BuildArtifacts, BuildError, BuildOptions, BuildResult, BuiltPackage,
};

const STAMP_SCHEMA: u32 = 1;
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

/// Shared `base-devel` template and the disposable roots derived from it.
pub(crate) fn build_chroot(
    source_dir: &Path,
    options: &BuildOptions,
) -> BuildResult<BuildArtifacts> {
    if !command_exists("arch-chroot") {
        return Err(BuildError::Command(
            "clean builds require arch-chroot (arch-install-scripts)".into(),
        ));
    }

    let build_uid = build_user_uid();
    let artifact_uid = artifact_user_uid();
    let template_dir = template_dir(options);
    let template_root = template_dir.join("root");
    let stamp_path = template_dir.join("template.json");

    let lock = open_template_lock(&template_dir)?;
    ensure_template(&template_root, &stamp_path, options, &lock)?;
    cleanup_stale_works(&template_dir);

    let work = template_dir.join(format!("work-{}-{}", std::process::id(), now_epoch_nanos()));
    run_root("mkdir", [OsStr::new("-p"), work.as_os_str()])?;
    // Held for the lifetime of the build so stale-work cleanup never removes
    // a root that is still in use.
    let _work_lock = open_file_lock(&work.join(".lock"))?;
    let kind = clone_template(&template_root, &work, &lock)?;
    lock.unlock();

    let guard = WorkGuard::new(work.clone(), kind);
    prepare_work(&work, source_dir, options, build_uid)?;
    run_makepkg(&work, options)?;
    let packages = collect_chroot_artifacts(&work, options, artifact_uid)?;
    drop(guard);

    if packages.is_empty() {
        return Err(BuildError::NoArtifacts(
            options.pkgdest.display().to_string(),
        ));
    }

    Ok(BuildArtifacts {
        source_dir: source_dir.to_path_buf(),
        pkgbase: crate::read_pkgbase(source_dir),
        packages,
    })
}

fn template_dir(options: &BuildOptions) -> PathBuf {
    options
        .chroot_dir
        .join(format!("template-{}", current_arch()))
}

// ── Template lifecycle ──────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TemplateStamp {
    schema: u32,
    arch: String,
    #[serde(default)]
    xpm_version: String,
    #[serde(default)]
    config_hash: String,
    created_at: u64,
    refreshed_at: u64,
}

impl TemplateStamp {
    fn new(arch: &str, now: u64) -> Self {
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
enum TemplateAction {
    Fresh,
    Refresh,
    Recreate,
}

fn ensure_template(
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

fn template_action(root: &Path, stamp_path: &Path, options: &BuildOptions) -> TemplateAction {
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

fn template_is_ready(root: &Path) -> bool {
    root.join("usr/bin/bash").is_file()
        && root.join("var/lib/xpm/local").is_dir()
        && root.join("etc/xpm.conf").is_file()
}

fn create_template(root: &Path) -> BuildResult<()> {
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

fn refresh_template(root: &Path, options: &BuildOptions) -> BuildResult<()> {
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

    // The template carries a public copy of the host keyring. Refresh it so a
    // stale template can still verify current database signatures.
    copy_host_keyring(root)?;
    run(&["sync", "--force"])?;
    run(&["install", "--needed", "--", "archlinux-keyring"])?;
    run(&["upgrade"])?;
    // A group can gain members between snapshots; `upgrade` does not install
    // packages that were never present.
    run(&["install", "--needed", "--", "base-devel"])?;
    tracing::info!(template = %root.display(), ttl_days = options.chroot_ttl_days, "base-devel template refreshed");
    Ok(())
}

fn read_stamp(path: &Path) -> Option<TemplateStamp> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_stamp(path: &Path, stamp: &TemplateStamp) -> BuildResult<()> {
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

struct TemplateLock {
    file: File,
}

impl TemplateLock {
    fn exclusive(&self) -> BuildResult<()> {
        self.flock(libc::LOCK_EX)
    }

    fn shared(&self) -> BuildResult<()> {
        self.flock(libc::LOCK_SH)
    }

    fn unlock(&self) {
        let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
    }

    fn flock(&self, operation: libc::c_int) -> BuildResult<()> {
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

fn open_template_lock(template_dir: &Path) -> BuildResult<TemplateLock> {
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

// ── Cheap disposable copies ─────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CopyKind {
    Snapshot,
    Reflink,
    Overlay,
    Copy,
}

impl CopyKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Reflink => "reflink",
            Self::Overlay => "overlay",
            Self::Copy => "copy",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "snapshot" => Some(Self::Snapshot),
            "reflink" => Some(Self::Reflink),
            "overlay" => Some(Self::Overlay),
            "copy" => Some(Self::Copy),
            _ => None,
        }
    }
}

fn clone_template(template: &Path, work: &Path, lock: &TemplateLock) -> BuildResult<CopyKind> {
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

fn copy_host_keyring(root: &Path) -> BuildResult<()> {
    let host = Path::new("/etc/pacman.d/gnupg");
    if !host.is_dir() {
        return Ok(());
    }
    let target = root.join("etc/pacman.d/gnupg");
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

fn xpm_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Hash the host configuration that `xpm bootstrap` copies into the template.
/// A changed repository list must recreate the template, not just upgrade it.
fn config_hash() -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for path in [
        Path::new("/etc/xpm.conf"),
        Path::new("/etc/pacman.conf"),
        Path::new("/etc/pacman.d/mirrorlist"),
    ] {
        if let Ok(bytes) = fs::read(path) {
            path.hash(&mut hasher);
            bytes.hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

fn is_btrfs_subvolume(path: &Path) -> bool {
    root_command("btrfs")
        .args(["subvolume", "show"])
        .arg(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn snapshot(template: &Path, root: &Path) -> BuildResult<bool> {
    let status = root_command("btrfs")
        .args(["subvolume", "snapshot"])
        .arg(template)
        .arg(root)
        .status()
        .map_err(|err| BuildError::Command(format!("btrfs snapshot: {err}")))?;
    Ok(status.success())
}

fn reflink(template: &Path, root: &Path) -> bool {
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

fn overlay(template: &Path, work: &Path, root: &Path) -> bool {
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

fn full_copy(template: &Path, root: &Path) -> BuildResult<()> {
    remove_tree(root);
    run_root(
        "cp",
        [OsStr::new("-a"), template.as_os_str(), root.as_os_str()],
    )
}

// ── Disposable root preparation ─────────────────────────────────────────────

fn prepare_work(
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
            command.arg("/preinstall").arg(name);
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

fn setup_build_user(root: &Path, build_uid: u32) -> BuildResult<()> {
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

fn install_helpers(root: &Path) -> BuildResult<()> {
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
    fs::write(&shim_source, include_str!("pacman-shim.sh")).map_err(|source| BuildError::Io {
        path: shim_source.display().to_string(),
        source,
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

fn mount_shared_paths(root: &Path, options: &BuildOptions, build_uid: u32) -> BuildResult<()> {
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

fn bind_mount(source: &Path, target: &Path, read_only: bool) -> BuildResult<()> {
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

fn run_makepkg(work: &Path, options: &BuildOptions) -> BuildResult<()> {
    let root = work.join("root");
    let makepkg = makepkg_line(options);
    let environment = makepkg_environment(options);
    let command = format!("cd /build && {environment} {makepkg}");
    tracing::info!(chroot = %root.display(), "building in clean chroot");
    let status = root_command("arch-chroot")
        .arg(&root)
        .args(["runuser", "-u", "build", "--", "bash", "-lc", &command])
        .status()
        .map_err(|err| BuildError::Command(format!("arch-chroot: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "clean chroot build exited with status {:?}",
            status.code()
        )));
    }
    Ok(())
}

fn makepkg_line(options: &BuildOptions) -> String {
    let mut makepkg = String::from("makepkg --noconfirm --cleanbuild --force --noprogressbar");
    if options.chroot_nodeps {
        makepkg.push_str(" --nodeps");
    }
    if !options.run_checks {
        makepkg.push_str(" --nocheck");
    }
    for arg in &options.extra_args {
        makepkg.push(' ');
        makepkg.push_str(&shell_quote(arg));
    }
    makepkg
}

fn makepkg_environment(options: &BuildOptions) -> String {
    let mut environment = String::from(
        "env -u LDFLAGS -u CFLAGS -u CXXFLAGS HOME=/home/build \
         PACMAN=/usr/local/bin/pacman PKGDEST=/pkgdest PKGEXT=.pkg.tar.zst",
    );
    if options.srcdest.is_some() {
        environment.push_str(" SRCDEST=/srcdest SRCPKGDEST=/srcdest");
    }
    if options.builddir.is_some() {
        environment.push_str(" BUILDDIR=/builddir");
    }
    environment
}

fn collect_chroot_artifacts(
    work: &Path,
    options: &BuildOptions,
    artifact_uid: u32,
) -> BuildResult<Vec<BuiltPackage>> {
    let pkgdest = work.join("root/pkgdest");
    fs::create_dir_all(&options.pkgdest).map_err(|source| BuildError::Io {
        path: options.pkgdest.display().to_string(),
        source,
    })?;
    let entries = match fs::read_dir(&pkgdest) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(BuildError::Io {
                path: pkgdest.display().to_string(),
                source,
            });
        }
    };
    for entry in entries.flatten() {
        let source = entry.path();
        let Some(name) = source.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".pkg.tar.zst") {
            continue;
        }
        let dest = options.pkgdest.join(name);
        fs::copy(&source, &dest).map_err(|source_error| BuildError::Io {
            path: dest.display().to_string(),
            source: source_error,
        })?;
        let _ = root_command("chown")
            .arg(format!("{artifact_uid}:{artifact_uid}"))
            .arg(&dest)
            .status();
    }
    let packages = collect_packages(&options.pkgdest, None)?;
    Ok(packages)
}

// ── Cleanup ─────────────────────────────────────────────────────────────────

struct WorkGuard {
    work: PathBuf,
    kind: CopyKind,
    armed: bool,
}

impl WorkGuard {
    fn new(work: PathBuf, kind: CopyKind) -> Self {
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

fn cleanup_work(work: &Path, kind: CopyKind) {
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

fn cleanup_stale_works(template_dir: &Path) {
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

fn write_marker(work: &Path, name: &str, value: &str) -> BuildResult<()> {
    write_root_file(&work.join(name), value.as_bytes())
}

fn write_root_file(path: &Path, bytes: &[u8]) -> BuildResult<()> {
    let mut child = root_command("tee")
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|err| BuildError::Command(format!("write {}: {err}", path.display())))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(bytes)
            .map_err(|err| BuildError::Command(format!("write {}: {err}", path.display())))?;
    }
    let status = child
        .wait()
        .map_err(|err| BuildError::Command(format!("write {}: {err}", path.display())))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "could not write {}",
            path.display()
        )));
    }
    Ok(())
}

fn open_file_lock(path: &Path) -> BuildResult<File> {
    if !path.exists() {
        let _ = root_command("touch").arg(path).status();
        let _ = root_command("chmod").args(["0644"]).arg(path).status();
    }
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .or_else(|_| OpenOptions::new().read(true).write(true).open(path))
        .map_err(|source| BuildError::Io {
            path: path.display().to_string(),
            source,
        })?;
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if locked != 0 {
        let source = std::io::Error::last_os_error();
        return Err(BuildError::Command(format!(
            "could not lock {}: {source}",
            path.display()
        )));
    }
    Ok(file)
}

// ── Small helpers ───────────────────────────────────────────────────────────

fn ensure_chroot_mountpoints(root: &Path) -> BuildResult<()> {
    for dir in ["proc", "sys", "dev", "run", "tmp"] {
        run_root("mkdir", [OsStr::new("-p"), root.join(dir).as_os_str()])?;
    }
    Ok(())
}

fn is_mountpoint(path: &Path) -> bool {
    Command::new("mountpoint")
        .arg("-q")
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_root<const N: usize>(program: &str, args: [&OsStr; N]) -> BuildResult<()> {
    let status = root_command(program)
        .args(args)
        .status()
        .map_err(|err| BuildError::Command(format!("{program}: {err}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(BuildError::Command(format!(
            "{program} exited with status {:?}",
            status.code()
        )))
    }
}

fn remove_tree(path: &Path) {
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

pub(crate) fn root_command(program: impl AsRef<OsStr>) -> Command {
    if effective_uid() == 0 {
        Command::new(program)
    } else {
        let mut command = Command::new("sudo");
        command.arg(program);
        command
    }
}

fn current_xpm() -> BuildResult<PathBuf> {
    std::env::current_exe()
        .map_err(|err| BuildError::Command(format!("resolve the xpm executable: {err}")))
}

fn command_exists(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program} >/dev/null 2>&1"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(crate) fn effective_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .unwrap_or(0)
}

fn build_user_uid() -> u32 {
    invoking_uid().unwrap_or(1000)
}

fn artifact_user_uid() -> u32 {
    if effective_uid() == 0 {
        invoking_uid().unwrap_or(0)
    } else {
        effective_uid()
    }
}

fn invoking_uid() -> Option<u32> {
    std::env::var("SUDO_UID")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|uid| *uid != 0)
}

fn current_arch() -> String {
    std::env::consts::ARCH.to_string()
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn now_epoch_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> BuildOptions {
        BuildOptions::new("/tmp/xpm-test-packages")
    }

    #[test]
    fn makepkg_line_adds_nodeps_and_quotes_extra_args() {
        let mut options = options();
        options.chroot_nodeps = true;
        options.run_checks = false;
        options.extra_args = vec!["--config".into(), "/tmp/a b.conf".into()];
        let line = makepkg_line(&options);
        assert!(line.contains("--nodeps"));
        assert!(line.contains("--nocheck"));
        assert!(line.contains("'/tmp/a b.conf'"));
    }

    #[test]
    fn makepkg_environment_forces_pkgext_and_forwards_caches() {
        let mut options = options();
        let env = makepkg_environment(&options);
        assert!(env.contains("PKGEXT=.pkg.tar.zst"));
        assert!(!env.contains("SRCDEST=/srcdest"));
        assert!(!env.contains("BUILDDIR=/builddir"));

        options.srcdest = Some("/tmp/sources".into());
        options.builddir = Some("/tmp/build".into());
        let env = makepkg_environment(&options);
        assert!(env.contains("SRCDEST=/srcdest"));
        assert!(env.contains("BUILDDIR=/builddir"));
    }

    #[test]
    fn copy_kind_round_trips() {
        for kind in [
            CopyKind::Snapshot,
            CopyKind::Reflink,
            CopyKind::Overlay,
            CopyKind::Copy,
        ] {
            assert_eq!(CopyKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(CopyKind::parse("other"), None);
    }

    #[test]
    fn template_freshness_honors_ttl_refresh_and_arch() {
        let dir = std::env::temp_dir().join(format!(
            "xpm-template-test-{}-{}",
            std::process::id(),
            now_epoch_nanos()
        ));
        let root = dir.join("root");
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::create_dir_all(root.join("var/lib/xpm/local")).unwrap();
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(root.join("usr/bin/bash"), b"").unwrap();
        std::fs::write(root.join("etc/xpm.conf"), b"").unwrap();

        let stamp_path = dir.join("template.json");
        let stamp = TemplateStamp::new(&current_arch(), now_epoch());
        write_stamp(&stamp_path, &stamp).unwrap();

        let mut options = options();
        assert_eq!(
            template_action(&root, &stamp_path, &options),
            TemplateAction::Fresh
        );

        options.chroot_refresh = true;
        assert_ne!(
            template_action(&root, &stamp_path, &options),
            TemplateAction::Fresh
        );

        options.chroot_refresh = false;
        options.chroot_ttl_days = 0;
        assert_ne!(
            template_action(&root, &stamp_path, &options),
            TemplateAction::Fresh
        );

        options.chroot_ttl_days = 7;
        let stale = TemplateStamp {
            refreshed_at: now_epoch().saturating_sub(8 * SECONDS_PER_DAY),
            ..stamp
        };
        write_stamp(&stamp_path, &stale).unwrap();
        assert_ne!(
            template_action(&root, &stamp_path, &options),
            TemplateAction::Fresh
        );

        let wrong_arch = TemplateStamp::new("not-this-arch", now_epoch());
        write_stamp(&stamp_path, &wrong_arch).unwrap();
        options.chroot_ttl_days = 7;
        assert_eq!(
            template_action(&root, &stamp_path, &options),
            TemplateAction::Recreate
        );

        let changed_config = TemplateStamp {
            config_hash: "different".into(),
            ..TemplateStamp::new(&current_arch(), now_epoch())
        };
        write_stamp(&stamp_path, &changed_config).unwrap();
        assert_eq!(
            template_action(&root, &stamp_path, &options),
            TemplateAction::Recreate
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
