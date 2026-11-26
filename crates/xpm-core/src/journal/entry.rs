//! Journal package entries.

use super::*;

/// One package affected by a transaction.
///
/// `repo`, `sha256` and `source` are provenance fields filled when the package
/// comes from a repository. They are all optional so schema-1 journals written
/// before they existed keep parsing unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalPackage {
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Repository the package was downloaded from (`None` for local files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// SHA-256 of the downloaded package file, when the sync DB declared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Mirror URL the package was downloaded from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl JournalPackage {
    pub fn install(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from: None,
            to: Some(version.into()),
            repo: None,
            sha256: None,
            source: None,
        }
    }

    pub fn remove(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from: Some(version.into()),
            to: None,
            repo: None,
            sha256: None,
            source: None,
        }
    }

    pub fn upgrade(
        name: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            from: Some(from.into()),
            to: Some(to.into()),
            repo: None,
            sha256: None,
            source: None,
        }
    }

    /// Attaches repository provenance to the entry.
    pub fn with_repo(mut self, repo: impl Into<String>) -> Self {
        self.repo = Some(repo.into());
        self
    }

    /// Attaches the downloaded file checksum to the entry.
    pub fn with_sha256(mut self, sha256: impl Into<String>) -> Self {
        self.sha256 = Some(sha256.into());
        self
    }

    /// Attaches the download source URL to the entry.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Human-readable form: `name 1.0-1`, `name 1.0-1 -> 1.1-1` or
    /// `name 1.0-1 (removed)`.
    pub fn describe(&self) -> String {
        match (&self.from, &self.to) {
            (Some(from), Some(to)) => format!("{} {from} -> {to}", self.name),
            (None, Some(to)) => format!("{} {to}", self.name),
            (Some(from), None) => format!("{} {from} (removed)", self.name),
            (None, None) => self.name.clone(),
        }
    }
}
