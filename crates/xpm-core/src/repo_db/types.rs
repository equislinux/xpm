//! Repository database entry types.

use super::*;

/// A package entry stored in a repository database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    pub version: String,
    pub filename: Option<String>,
    pub sha256sum: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub arch: Option<String>,
    pub depends: Vec<String>,
    pub opt_depends: Vec<String>,
    pub provides: Vec<String>,
    pub conflicts: Vec<String>,
    pub replaces: Vec<String>,
    pub groups: Vec<String>,
    pub files: Vec<String>,
}

impl RepoEntry {
    /// File name to download for this entry.
    ///
    /// xpkg-extended databases carry `FILENAME`; standard Arch databases do
    /// not, so the Arch convention `name-version-arch.pkg.tar.zst` is derived
    /// from the entry (falling back to the requested architecture for `any`).
    pub fn resolved_filename(&self, default_arch: &str) -> String {
        if let Some(filename) = &self.filename {
            return filename.clone();
        }
        let arch = self
            .arch
            .as_deref()
            .map(str::trim)
            .filter(|arch| !arch.is_empty() && *arch != "any")
            .unwrap_or(default_arch);
        format!("{}-{}-{arch}.pkg.tar.zst", self.name, self.version)
    }
}

/// Parsed sync database for a repository (e.g. `core.db`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncDb {
    pub repo: String,
    pub entries: Vec<RepoEntry>,
}

/// In-memory representation of the local package database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalDb {
    pub entries: Vec<RepoEntry>,
}
