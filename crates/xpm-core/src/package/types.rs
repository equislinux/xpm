//! Types for parsed package metadata.

use std::collections::HashMap;
use std::path::PathBuf;

// ── PackageMeta ───────────────────────────────────────────────

/// Parsed contents of a `.PKGINFO` file.
#[derive(Debug, Clone, Default)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub release: String,
    pub description: String,
    pub url: String,
    pub build_date: u64,
    pub installed_size: u64,
    pub arch: Vec<String>,
    pub license: Vec<String>,
    pub depends: Vec<String>,
    pub makedepends: Vec<String>,
    pub checkdepends: Vec<String>,
    pub optdepends: Vec<String>,
    pub provides: Vec<String>,
    pub conflicts: Vec<String>,
    pub replaces: Vec<String>,
    /// Configuration files (`backup = etc/foo.conf`). These get `.pacnew`
    /// handling on install/upgrade and `.pacsave` handling on removal.
    pub backup: Vec<String>,
    /// Any additional key-value pairs not covered above.
    pub extra: HashMap<String, Vec<String>>,
}

impl PackageMeta {
    /// Full version string: `{version}-{release}`.
    pub fn full_version(&self) -> String {
        format!("{}-{}", self.version, self.release)
    }
}

// ── BuildInfo ─────────────────────────────────────────────────

/// Parsed contents of a `.BUILDINFO` file.
#[derive(Debug, Clone, Default)]
pub struct BuildInfo {
    pub pkgname: String,
    pub pkgver: String,
    pub builddate: u64,
    pub builddir: String,
    pub buildtool: String,
    pub buildhost: String,
    pub compress: String,
    pub compress_level: i32,
    pub cflags: String,
    pub cxxflags: String,
    pub ldflags: String,
    pub makeflags: String,
}

// ── MTREE types ───────────────────────────────────────────────

/// File type recorded in an `.MTREE` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MtreeFileType {
    File,
    Dir,
    Link,
}

/// A single entry from the `.MTREE` manifest.
#[derive(Debug, Clone)]
pub struct MtreeEntry {
    pub path: PathBuf,
    pub file_type: MtreeFileType,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub size: u64,
    /// SHA-256 hex digest (only for regular files).
    pub sha256: Option<String>,
    /// Symlink target (only for links).
    pub link_target: Option<String>,
}

// ── PackageFile ───────────────────────────────────────────────

/// A file entry listed inside a package archive.
#[derive(Debug, Clone)]
pub struct PackageFile {
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_version_format() {
        let meta = PackageMeta {
            version: "1.2.3".into(),
            release: "1".into(),
            ..Default::default()
        };
        assert_eq!(meta.full_version(), "1.2.3-1");
    }

    #[test]
    fn default_package_meta_is_empty() {
        let meta = PackageMeta::default();
        assert!(meta.name.is_empty());
        assert!(meta.depends.is_empty());
        assert!(meta.extra.is_empty());
    }
}
