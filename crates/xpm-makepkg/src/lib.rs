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
            extra_args: Vec::new(),
        }
    }
}

/// Run makepkg in `source_dir` and return the produced package files.
pub fn build(source_dir: &Path, options: &BuildOptions) -> BuildResult<BuildArtifacts> {
    if is_effective_root() {
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
