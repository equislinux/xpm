//! `makepkg` orchestration for xpm.
//!
//! This crate is intentionally small: it validates a source checkout, invokes
//! `makepkg` with a deterministic environment, and reports the package files it
//! produced. It never calls pacman and never installs anything; xpm decides what
//! to do with the artifacts.

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
/// devtools is deliberately not required: pacstrap, arch-chroot and
/// systemd-nspawn are part of arch-install-scripts/systemd, which are already
/// present on X installs. The pristine root is created once; every build uses
/// a disposable copy and a bind-mounted package directory.
fn build_chroot(source_dir: &Path, options: &BuildOptions) -> BuildResult<BuildArtifacts> {
    if !command_exists("pacstrap") || !command_exists("systemd-nspawn") {
        return Err(BuildError::Command(
            "clean builds require pacstrap (arch-install-scripts) and systemd-nspawn".into(),
        ));
    }

    let uid = effective_uid();
    let build_uid = if uid == 0 { 1000 } else { uid };
    let root = options.chroot_dir.join("root");
    if !root.is_dir() {
        std::fs::create_dir_all(&root).map_err(|source| BuildError::Io {
            path: root.display().to_string(),
            source,
        })?;
        tracing::info!(chroot = %root.display(), "creating clean build chroot");
        let status = root_command("pacstrap")
            .args(["-c", "-C", "/etc/pacman.conf"])
            .arg(&root)
            .arg("base-devel")
            .status()
            .map_err(|err| BuildError::Command(format!("pacstrap: {err}")))?;
        if !status.success() {
            return Err(BuildError::Command(format!(
                "pacstrap exited with status {:?}",
                status.code()
            )));
        }
        let setup = format!(
            "useradd -m -u {build_uid} -s /bin/bash build 2>/dev/null || true; \
             printf '%s ALL=(ALL) NOPASSWD: ALL\\n' build > /etc/sudoers.d/build; \
             chmod 440 /etc/sudoers.d/build"
        );
        let status = root_command("arch-chroot")
            .arg(&root)
            .args(["bash", "-lc", &setup])
            .status()
            .map_err(|err| BuildError::Command(format!("arch-chroot: {err}")))?;
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
    let _ = std::fs::remove_dir_all(&work);
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

    let build_dir = work.join("build");
    let status = root_command("mkdir")
        .args(["-p"])
        .arg(&build_dir)
        .status()
        .map_err(|err| BuildError::Command(format!("mkdir: {err}")))?;
    if !status.success() {
        return Err(BuildError::Command("could not create chroot build directory".into()));
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

    std::fs::create_dir_all(&options.pkgdest).map_err(|source| BuildError::Io {
        path: options.pkgdest.display().to_string(),
        source,
    })?;
    if uid == 0 {
        let _ = root_command("chown")
            .args(["-R", &format!("{build_uid}:{build_uid}")])
            .arg(&options.pkgdest)
            .status();
    }
    let pkgdest = options
        .pkgdest
        .canonicalize()
        .map_err(|source| BuildError::Io {
            path: options.pkgdest.display().to_string(),
            source,
        })?;

    let mut makepkg =
        String::from("makepkg --syncdeps --noconfirm --cleanbuild --force --noprogressbar");
    if !options.run_checks {
        makepkg.push_str(" --nocheck");
    }
    for arg in &options.extra_args {
        makepkg.push(' ');
        makepkg.push_str(&shell_quote(arg));
    }
    let command = format!("PKGDEST=/pkgdest {makepkg}");
    let mut spawn = root_command("systemd-nspawn");
    spawn
        .args(["-D"])
        .arg(&work)
        .arg(format!("--bind={}:/pkgdest", pkgdest.display()))
        .arg("--network-namespace-path=/proc/1/ns/net")
        .args(["-u", "build", "--chdir=/build", "/bin/bash", "-lc"])
        .arg(command);
    tracing::info!(chroot = %options.chroot_dir.display(), "building in clean chroot");
    let status = spawn
        .status()
        .map_err(|err| BuildError::Command(format!("systemd-nspawn: {err}")))?;
    let cleanup = root_command("rm").args(["-rf"]).arg(&work).status();
    let _ = cleanup;
    if !status.success() {
        return Err(BuildError::Command(format!(
            "clean chroot build exited with status {:?}",
            status.code()
        )));
    }

    let packages = collect_packages(&pkgdest, None)?;
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
