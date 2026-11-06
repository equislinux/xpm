//! `makepkg` orchestration for xpm.
//!
//! This crate is intentionally small: it validates a source checkout, invokes
//! `makepkg` with a deterministic environment, and reports the package files it
//! produced. It never calls pacman and never installs anything; xpm decides what
//! to do with the artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("source package must be built as a normal user, not root")]
    RunningAsRoot,

    #[error("no PKGBUILD found in {0}")]
    MissingPkgbuild(String),

    #[error("could not create package directory {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to run makepkg: {0}")]
    Command(String),

    #[error("makepkg produced no package artifacts in {0}")]
    NoArtifacts(String),

    #[error("invalid package filename: {0}")]
    InvalidPackageFilename(String),
}

pub type BuildResult<T> = Result<T, BuildError>;

/// One package file produced by makepkg.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuiltPackage {
    pub name: String,
    pub path: PathBuf,
}

/// All artifacts produced by one package base.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BuildArtifacts {
    pub source_dir: PathBuf,
    pub pkgbase: String,
    pub packages: Vec<BuiltPackage>,
}

impl BuildArtifacts {
    pub fn get(&self, name: &str) -> Option<&BuiltPackage> {
        self.packages.iter().find(|package| package.name == name)
    }
}

/// Options for one makepkg invocation.
#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub pkgdest: PathBuf,
    pub srcdest: Option<PathBuf>,
    pub builddir: Option<PathBuf>,
    pub run_checks: bool,
    pub clean: bool,
    /// Build inside a clean devtools chroot (`makechrootpkg`).
    pub chroot: bool,
    /// Directory containing the pristine `root` and disposable build copies.
    pub chroot_dir: PathBuf,
    /// Local packages to install into the clean chroot before building.
    pub preinstall: Vec<PathBuf>,
    pub extra_args: Vec<String>,
}

impl BuildOptions {
    pub fn new(pkgdest: impl Into<PathBuf>) -> Self {
        Self {
            pkgdest: pkgdest.into(),
            srcdest: None,
            builddir: None,
            run_checks: true,
            clean: false,
            chroot: false,
            chroot_dir: PathBuf::from("/var/lib/archbuild/xpm-aur"),
            preinstall: Vec::new(),
            extra_args: Vec::new(),
        }
    }
}

/// Run makepkg in `source_dir` and return the produced package files.
pub fn build(source_dir: &Path, options: &BuildOptions) -> BuildResult<BuildArtifacts> {
    if is_effective_root() && !options.chroot {
        return Err(BuildError::RunningAsRoot);
    }
    if !source_dir.join("PKGBUILD").is_file() {
        return Err(BuildError::MissingPkgbuild(
            source_dir.display().to_string(),
        ));
    }

    if options.clean && options.pkgdest.exists() {
        std::fs::remove_dir_all(&options.pkgdest).map_err(|source| BuildError::Io {
            path: options.pkgdest.display().to_string(),
            source,
        })?;
    }
    std::fs::create_dir_all(&options.pkgdest).map_err(|source| BuildError::Io {
        path: options.pkgdest.display().to_string(),
        source,
    })?;

    if options.chroot {
        return build_chroot(source_dir, options);
    }

    let pkgbase = read_pkgbase(source_dir);
    let mut command = std::process::Command::new("makepkg");
    command
        .current_dir(source_dir)
        .arg("--nodeps")
        .arg("--noconfirm")
        .arg("--cleanbuild")
        .arg("--force")
        .arg("--noprogressbar")
        .env("PKGDEST", &options.pkgdest)
        .env("PKGEXT", ".pkg.tar.zst");
    if !options.run_checks {
        command.arg("--nocheck");
    }
    if let Some(srcdest) = &options.srcdest {
        command.env("SRCDEST", srcdest).env("SRCPKGDEST", srcdest);
    }
    if let Some(builddir) = &options.builddir {
        command.env("BUILDDIR", builddir);
    }
    command.args(&options.extra_args);

    tracing::debug!(?command, "running makepkg");
    let status = command
        .status()
        .map_err(|err| BuildError::Command(err.to_string()))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "makepkg exited with status {:?}",
            status.code()
        )));
    }

    let mut packages = Vec::new();
    let entries = std::fs::read_dir(&options.pkgdest).map_err(|source| BuildError::Io {
        path: options.pkgdest.display().to_string(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| BuildError::Io {
            path: options.pkgdest.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !filename.ends_with(".pkg.tar.zst") {
            continue;
        }
        let name = package_name_from_filename(filename)?;
        packages.push(BuiltPackage { name, path });
    }

    if packages.is_empty() {
        return Err(BuildError::NoArtifacts(
            options.pkgdest.display().to_string(),
        ));
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(BuildArtifacts {
        source_dir: source_dir.to_path_buf(),
        pkgbase,
        packages,
    })
}

/// Build in a clean chroot assembled with pacstrap.
///
/// devtools is deliberately not required: pacstrap and arch-chroot are part of
/// arch-install-scripts. The pristine root is created once; every build uses a
/// disposable copy and runs makepkg as an unprivileged build user.
fn build_chroot(source_dir: &Path, options: &BuildOptions) -> BuildResult<BuildArtifacts> {
    if !command_exists("pacstrap") || !command_exists("arch-chroot") {
        return Err(BuildError::Command(
            "clean builds require pacstrap and arch-chroot (arch-install-scripts)".into(),
        ));
    }

    let uid = effective_uid();
    let build_uid = if uid == 0 { 1000 } else { uid };
    let root = options.chroot_dir.join("root");
    if !root.is_dir() {
        let status = root_command("mkdir")
            .args(["-p"])
            .arg(&root)
            .status()
            .map_err(|err| BuildError::Command(format!("mkdir: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(format!(
                "could not create chroot root {}",
                root.display()
            )));
        }
        tracing::info!(chroot = %root.display(), "creating clean build chroot with xpm");
        // Do not use pacstrap: xpm can bootstrap its own empty root from the
        // same signed repositories and keyring.
        let status = root_command("xpm")
            .args(["--no-confirm", "--no-pacman-mirror", "--root"])
            .arg(&root)
            .args(["bootstrap", "base-devel"])
            .status()
            .map_err(|err| BuildError::Command(format!("xpm bootstrap: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(format!(
                "xpm bootstrap exited with status {:?}",
                status.code()
            )));
        }
        ensure_chroot_mountpoints(&root)?;
        let setup = format!(
            "useradd -m -u {build_uid} -s /bin/bash build 2>/dev/null || true; \
             printf '%s ALL=(ALL) NOPASSWD: ALL\\n' build > /etc/sudoers.d/build; \
             chmod 440 /etc/sudoers.d/build"
        );
        // arch-chroot warns and can misbehave when the root is not a mountpoint.
        let bound = !root_command("mountpoint")
            .arg("-q")
            .arg(&root)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if bound {
            let status = root_command("mount")
                .args(["--bind"])
                .arg(&root)
                .arg(&root)
                .status()
                .map_err(|err| BuildError::Command(format!("bind chroot root: {err}")))?;
            if !status.success() {
                return Err(BuildError::Command(
                    "could not bind-mount chroot root".into(),
                ));
            }
        }
        let status = root_command("arch-chroot")
            .arg(&root)
            .args(["bash", "-lc", &setup])
            .status()
            .map_err(|err| BuildError::Command(format!("arch-chroot: {err}")))?;
        if bound {
            let _ = root_command("umount").arg(&root).status();
        }
        if !status.success() {
            return Err(BuildError::Command(format!(
                "chroot user setup exited with status {:?}",
                status.code()
            )));
        }
    }

    let work = options
        .chroot_dir
        .join(format!("work-{}", std::process::id()));
    let _ = root_command("rm").args(["-rf"]).arg(&work).status();
    let status = root_command("cp")
        .args(["-a", "--reflink=auto"])
        .arg(&root)
        .arg(&work)
        .status()
        .map_err(|err| BuildError::Command(format!("chroot copy: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "chroot copy exited with status {:?}",
            status.code()
        )));
    }

    ensure_chroot_mountpoints(&work)?;

    // The chroot copy is root-owned. Install the shim and xpm with the same
    // privilege used to create it, not as the invoking xpm-aur user.
    let shim_source =
        std::env::temp_dir().join(format!("xpm-pacman-shim-{}.sh", std::process::id()));
    fs::write(&shim_source, include_str!("pacman-shim.sh"))
        .map_err(|err| BuildError::Command(format!("write temporary pacman shim: {err}")))?;
    let status = root_command("install")
        .args(["-Dm755"])
        .arg(&shim_source)
        .arg(work.join("usr/local/bin/pacman"))
        .status()
        .map_err(|err| BuildError::Command(format!("install pacman shim: {err}")))?;
    let _ = fs::remove_file(&shim_source);
    if !status.success() {
        return Err(BuildError::Command("could not install pacman shim".into()));
    }
    let current_xpm = std::env::current_exe()
        .map_err(|err| BuildError::Command(format!("current xpm: {err}")))?;
    let status = root_command("install")
        .args(["-Dm755"])
        .arg(&current_xpm)
        .arg(work.join("usr/local/bin/xpm"))
        .status()
        .map_err(|err| BuildError::Command(format!("install xpm into chroot: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(
            "could not install xpm into chroot".into(),
        ));
    }

    // pacman inside the fresh root needs a real package cache directory;
    // without it its free-space check cannot resolve the cachedir mount point.
    let cache_dir = work.join("var/cache/pacman/pkg");
    let status = root_command("mkdir")
        .args(["-p"])
        .arg(&cache_dir)
        .status()
        .map_err(|err| BuildError::Command(format!("mkdir: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(
            "could not create chroot pacman cache".into(),
        ));
    }

    // arch-chroot expects a mountpoint for pacman's free-space checks.
    let status = root_command("mount")
        .args(["--bind"])
        .arg(&work)
        .arg(&work)
        .status()
        .map_err(|err| BuildError::Command(format!("mount --bind: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(
            "could not bind-mount the chroot root".into(),
        ));
    }

    let build_dir = work.join("build");
    let status = root_command("mkdir")
        .args(["-p"])
        .arg(&build_dir)
        .status()
        .map_err(|err| BuildError::Command(format!("mkdir: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(
            "could not create chroot build directory".into(),
        ));
    }
    let _ = root_command("chown")
        .args(["-R", &format!("{build_uid}:{build_uid}")])
        .arg(&build_dir)
        .status();
    let status = root_command("cp")
        .args(["-a", "--reflink=auto"])
        .arg(format!("{}/.", source_dir.display()))
        .arg(&build_dir)
        .status()
        .map_err(|err| BuildError::Command(format!("source copy: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "source copy exited with status {:?}",
            status.code()
        )));
    }
    let _ = root_command("chown")
        .args(["-R", &format!("{build_uid}:{build_uid}")])
        .arg(&build_dir)
        .status();

    let pkgdest_in_chroot = work.join("pkgdest");
    let status = root_command("mkdir")
        .args(["-p"])
        .arg(&pkgdest_in_chroot)
        .status()
        .map_err(|err| BuildError::Command(format!("mkdir: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command(
            "could not create chroot package directory".into(),
        ));
    }
    let _ = root_command("chown")
        .args(["-R", &format!("{build_uid}:{build_uid}")])
        .arg(&pkgdest_in_chroot)
        .status();

    if !options.preinstall.is_empty() {
        let preinstall_dir = work.join("preinstall");
        let status = root_command("mkdir")
            .args(["-p"])
            .arg(&preinstall_dir)
            .status()
            .map_err(|err| BuildError::Command(format!("mkdir: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(
                "could not create chroot preinstall directory".into(),
            ));
        }
        for package in &options.preinstall {
            let status = root_command("cp")
                .arg(package)
                .arg(&preinstall_dir)
                .status()
                .map_err(|err| BuildError::Command(format!("copy preinstall: {err}")))?;
            if !status.success() {
                return Err(BuildError::Command(format!(
                    "could not copy {} into the chroot",
                    package.display()
                )));
            }
        }
        let status = root_command("arch-chroot")
            .arg(&work)
            .args([
                "bash",
                "-lc",
                "/usr/local/bin/xpm --no-confirm install /preinstall/*.pkg.tar.zst",
            ])
            .status()
            .map_err(|err| BuildError::Command(format!("preinstall pacman: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(
                "could not preinstall AUR dependencies into the chroot".into(),
            ));
        }
    }

    let mut makepkg =
        String::from("makepkg --syncdeps --noconfirm --cleanbuild --force --noprogressbar");
    if !options.run_checks {
        makepkg.push_str(" --nocheck");
    }
    for arg in &options.extra_args {
        makepkg.push(' ');
        makepkg.push_str(&shell_quote(arg));
    }
    let command = format!("env -u LDFLAGS -u CFLAGS -u CXXFLAGS PACMAN=/usr/local/bin/pacman PKGDEST=/pkgdest {makepkg}");
    tracing::info!(chroot = %options.chroot_dir.display(), "building in clean chroot");
    let status = root_command("arch-chroot")
        .arg(&work)
        .args(["runuser", "-u", "build", "--", "bash", "-lc"])
        .arg(format!("cd /build && {command}"))
        .status()
        .map_err(|err| BuildError::Command(format!("arch-chroot: {err}")))?;

    if status.success() {
        std::fs::create_dir_all(&options.pkgdest).map_err(|source| BuildError::Io {
            path: options.pkgdest.display().to_string(),
            source,
        })?;
        let entries = std::fs::read_dir(&pkgdest_in_chroot).map_err(|source| BuildError::Io {
            path: pkgdest_in_chroot.display().to_string(),
            source,
        })?;
        for entry in entries.flatten() {
            let source = entry.path();
            let Some(name) = source.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.ends_with(".pkg.tar.zst") {
                continue;
            }
            let dest = options.pkgdest.join(name);
            let _ = std::fs::copy(&source, &dest);
            let _ = root_command("chown")
                .arg(format!("{uid}:{uid}"))
                .arg(&dest)
                .status();
        }
    }
    let _ = root_command("umount").args(["-R"]).arg(&work).status();
    let _ = root_command("rm").args(["-rf"]).arg(&work).status();
    if !status.success() {
        return Err(BuildError::Command(format!(
            "clean chroot build exited with status {:?}",
            status.code()
        )));
    }

    let packages = collect_packages(&options.pkgdest, None)?;
    if packages.is_empty() {
        return Err(BuildError::NoArtifacts(
            options.pkgdest.display().to_string(),
        ));
    }
    Ok(BuildArtifacts {
        source_dir: source_dir.to_path_buf(),
        pkgbase: read_pkgbase(source_dir),
        packages,
    })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn collect_packages(
    dir: &Path,
    newer_than: Option<std::time::SystemTime>,
) -> BuildResult<Vec<BuiltPackage>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(BuildError::Io {
                path: dir.display().to_string(),
                source,
            });
        }
    };
    let mut packages = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| BuildError::Io {
            path: dir.display().to_string(),
            source,
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if let Some(cutoff) = newer_than {
            let modified = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if modified < cutoff {
                continue;
            }
        }
        let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !filename.ends_with(".pkg.tar.zst") {
            continue;
        }
        packages.push(BuiltPackage {
            name: package_name_from_filename(filename)?,
            path,
        });
    }
    Ok(packages)
}

fn command_exists(program: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program} >/dev/null 2>&1"))
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn effective_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .unwrap_or(0)
}

fn ensure_chroot_mountpoints(root: &Path) -> BuildResult<()> {
    for dir in ["proc", "sys", "dev", "run", "tmp"] {
        let status = root_command("mkdir")
            .args(["-p"])
            .arg(root.join(dir))
            .status()
            .map_err(|err| BuildError::Command(format!("mkdir {}: {err}", dir)))?;
        if !status.success() {
            return Err(BuildError::Command(format!(
                "could not create chroot mountpoint {dir}"
            )));
        }
    }
    Ok(())
}

fn root_command(program: &str) -> std::process::Command {
    if effective_uid() == 0 {
        std::process::Command::new(program)
    } else {
        let mut command = std::process::Command::new("sudo");
        command.arg(program);
        command
    }
}

/// Read `pkgbase` from `.SRCINFO`, falling back to the directory name.
fn read_pkgbase(source_dir: &Path) -> String {
    let path = source_dir.join(".SRCINFO");
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            if let Some(value) = line.trim().strip_prefix("pkgbase =") {
                let value = value.trim();
                if !value.is_empty() {
                    return value.to_string();
                }
            }
        }
    }
    source_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Extract `name` from `name-pkgver-pkgrel-arch.pkg.tar.zst`.
pub fn package_name_from_filename(filename: &str) -> BuildResult<String> {
    let Some(stem) = filename.strip_suffix(".pkg.tar.zst") else {
        return Err(BuildError::InvalidPackageFilename(filename.into()));
    };
    let mut parts = stem.rsplitn(4, '-');
    let _arch = parts.next();
    let _pkgrel = parts.next();
    let _pkgver = parts.next();
    match parts.next() {
        Some(name) if !name.is_empty() => Ok(name.to_string()),
        _ => Err(BuildError::InvalidPackageFilename(filename.into())),
    }
}

fn is_effective_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .map(|metadata| metadata.uid() == 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_with_hyphens() {
        assert_eq!(
            package_name_from_filename("foo-bar-1.2.3-4-x86_64.pkg.tar.zst").unwrap(),
            "foo-bar"
        );
        assert!(package_name_from_filename("bad.tar.zst").is_err());
    }

    #[test]
    fn reads_pkgbase_from_srcinfo() {
        let dir = std::env::temp_dir().join(format!(
            "xpm-makepkg-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".SRCINFO"), "pkgbase = demo\n").unwrap();
        assert_eq!(read_pkgbase(&dir), "demo");
        let _ = std::fs::remove_dir_all(dir);
    }
}
