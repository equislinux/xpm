use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;

// ── Default paths ───────────────────────────────────────────────────────────

const DEFAULT_ROOT_DIR: &str = "/";
const DEFAULT_DB_PATH: &str = "/var/lib/xpm/";
const DEFAULT_CACHE_DIR: &str = "/var/cache/xpm/pkg/";
const DEFAULT_LOG_FILE: &str = "/var/log/xpm.log";
/// Keyring shared with pacman (X Linux ships pacman's keyring).
pub const DEFAULT_GPG_DIR: &str = "/etc/pacman.d/gnupg/";
/// Native xpm keyring, used when the pacman one is absent (or by preference
/// through the `gpg_dir` option).
pub const XPM_GPG_DIR: &str = "/etc/xpm/gnupg/";
const DEFAULT_CONFIG_PATH: &str = "/etc/xpm.conf";

// ── Configuration structs ───────────────────────────────────────────────────

/// Signature verification level for packages and databases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SigLevel {
    /// Signature verification is mandatory.
    Required,
    /// Signature is checked if present, but not required.
    #[default]
    Optional,
    /// Signature verification is disabled.
    Never,
}

/// Repository configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct Repository {
    /// Repository name (e.g. "core", "extra", "community").
    pub name: String,
    /// Mirror URLs for this repository.
    #[serde(default)]
    pub server: Vec<String>,
    /// Signature verification level override for this repo.
    pub sig_level: Option<SigLevel>,
}

/// General options for the package manager.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GeneralOptions {
    /// Root directory for package installation.
    pub root_dir: PathBuf,
    /// Database directory path.
    pub db_path: PathBuf,
    /// Package cache directory.
    pub cache_dir: PathBuf,
    /// Log file location.
    pub log_file: PathBuf,
    /// GPG keyring directory.
    pub gpg_dir: PathBuf,
    /// Packages that should never be upgraded.
    pub hold_pkg: Vec<String>,
    /// Packages to ignore during upgrades.
    pub ignore_pkg: Vec<String>,
    /// Groups of packages to ignore during upgrades.
    pub ignore_group: Vec<String>,
    /// System architecture.
    pub architecture: Option<String>,
    /// Default signature verification level.
    pub sig_level: SigLevel,
    /// Use color in output.
    pub color: bool,
    /// Perform operations in parallel (number of threads).
    pub parallel_downloads: u32,
    /// Check available disk space before installing.
    pub check_space: bool,
}

impl GeneralOptions {
    /// Keyring directory actually used for signature verification.
    ///
    /// Resolution order: the configured `gpg_dir` when it exists, then the
    /// pacman keyring, then the native xpm keyring, and finally the configured
    /// value (which produces the most useful error message when nothing
    /// exists yet).
    pub fn effective_gpg_dir(&self) -> PathBuf {
        for candidate in [
            self.gpg_dir.as_path(),
            Path::new(DEFAULT_GPG_DIR),
            Path::new(XPM_GPG_DIR),
        ] {
            if candidate.is_dir() {
                return candidate.to_path_buf();
            }
        }
        self.gpg_dir.clone()
    }

    /// Path of the trusted-keys file inside [`Self::effective_gpg_dir`].
    pub fn keyring_path(&self) -> PathBuf {
        self.effective_gpg_dir().join("trustedkeys.gpg")
    }
}

impl Default for GeneralOptions {
    fn default() -> Self {
        Self {
            root_dir: PathBuf::from(DEFAULT_ROOT_DIR),
            db_path: PathBuf::from(DEFAULT_DB_PATH),
            cache_dir: PathBuf::from(DEFAULT_CACHE_DIR),
            log_file: PathBuf::from(DEFAULT_LOG_FILE),
            gpg_dir: PathBuf::from(DEFAULT_GPG_DIR),
            hold_pkg: Vec::new(),
            ignore_pkg: Vec::new(),
            ignore_group: Vec::new(),
            architecture: None,
            sig_level: SigLevel::Optional,
            color: true,
            parallel_downloads: 5,
            check_space: true,
        }
    }
}

/// Top-level xpm configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct XpmConfig {
    /// General package manager options.
    pub options: GeneralOptions,
    /// Configured repositories.
    #[serde(rename = "repo")]
    pub repositories: Vec<Repository>,
}

impl Default for XpmConfig {
    fn default() -> Self {
        Self {
            options: GeneralOptions::default(),
            repositories: vec![Repository {
                name: "x".to_string(),
                server: vec!["https://equislinux.github.io/x-repo/x/$arch".into()],
                sig_level: None,
            }],
        }
    }
}

impl XpmConfig {
    /// Load configuration from a TOML file.
    ///
    /// Falls back to defaults if the file does not exist and `allow_missing`
    /// is true.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Err(ConfigError::NotFound {
                path: path.to_path_buf(),
            });
        }

        let contents =
            std::fs::read_to_string(path).map_err(|e| ConfigError::ReadError { source: e })?;

        let config: Self =
            toml::from_str(&contents).map_err(|e| ConfigError::ParseError { source: e })?;

        config.validate()?;

        tracing::info!(path = %path.display(), "loaded configuration");
        Ok(config)
    }

    /// Load configuration, returning defaults if the file is not found.
    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        match Self::load(path) {
            Ok(config) => Ok(config),
            Err(ConfigError::NotFound { .. }) => {
                tracing::warn!(
                    path = %path.display(),
                    "configuration file not found, using defaults"
                );
                Ok(Self::default())
            }
            Err(e) => Err(e),
        }
    }

    /// Return the default configuration file path.
    pub fn default_path() -> PathBuf {
        PathBuf::from(DEFAULT_CONFIG_PATH)
    }

    /// Validate the configuration for logical consistency.
    fn validate(&self) -> Result<(), ConfigError> {
        if self.options.parallel_downloads == 0 {
            return Err(ConfigError::Validation {
                message: "parallel_downloads must be at least 1".into(),
            });
        }

        for repo in &self.repositories {
            if repo.name.is_empty() {
                return Err(ConfigError::Validation {
                    message: "repository name cannot be empty".into(),
                });
            }
            if repo.server.is_empty() {
                return Err(ConfigError::Validation {
                    message: format!("repository '{}' has no servers configured", repo.name),
                });
            }
        }

        Ok(())
    }

    /// Apply CLI overrides to the configuration (root_dir, db_path, etc.).
    pub fn apply_overrides(
        &mut self,
        root: Option<&Path>,
        db_path: Option<&Path>,
        cache_dir: Option<&Path>,
    ) {
        if let Some(root) = root {
            self.options.root_dir = root.to_path_buf();
        }
        if let Some(db) = db_path {
            self.options.db_path = db.to_path_buf();
        }
        if let Some(cache) = cache_dir {
            self.options.cache_dir = cache.to_path_buf();
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config() {
        let config = XpmConfig::default();
        assert_eq!(config.options.root_dir, PathBuf::from("/"));
        assert_eq!(config.options.db_path, PathBuf::from("/var/lib/xpm/"));
        assert_eq!(config.options.parallel_downloads, 5);
        assert_eq!(config.repositories.len(), 1);
        assert_eq!(config.repositories[0].name, "x");
    }

    #[test]
    fn test_load_from_toml() {
        let toml_content = r#"
[options]
root_dir = "/"
db_path = "/var/lib/xpm/"
cache_dir = "/var/cache/xpm/pkg/"
log_file = "/var/log/xpm.log"
gpg_dir = "/etc/pacman.d/gnupg/"
color = true
parallel_downloads = 3
check_space = true

[[repo]]
name = "core"
server = ["https://mirror.example.com/archlinux/core/os/x86_64"]

[[repo]]
name = "extra"
server = ["https://mirror.example.com/archlinux/extra/os/x86_64"]
"#;

        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        tmpfile.write_all(toml_content.as_bytes()).unwrap();

        let config = XpmConfig::load(tmpfile.path()).unwrap();
        assert_eq!(config.options.parallel_downloads, 3);
        assert_eq!(config.repositories.len(), 2);
        assert_eq!(config.repositories[0].name, "core");
    }

    #[test]
    fn test_load_missing_file_returns_error() {
        let result = XpmConfig::load(Path::new("/nonexistent/xpm.conf"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_or_default_missing_file() {
        let config = XpmConfig::load_or_default(Path::new("/nonexistent/xpm.conf")).unwrap();
        assert_eq!(config.options.root_dir, PathBuf::from("/"));
    }

    #[test]
    fn test_apply_overrides() {
        let mut config = XpmConfig::default();
        config.apply_overrides(
            Some(Path::new("/mnt/install")),
            Some(Path::new("/custom/db")),
            None,
        );
        assert_eq!(config.options.root_dir, PathBuf::from("/mnt/install"));
        assert_eq!(config.options.db_path, PathBuf::from("/custom/db"));
        // cache_dir should remain default
        assert_eq!(
            config.options.cache_dir,
            PathBuf::from("/var/cache/xpm/pkg/")
        );
    }

    #[test]
    fn keyring_path_prefers_existing_configured_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let options = GeneralOptions {
            gpg_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };

        assert_eq!(options.effective_gpg_dir(), tmp.path());
        assert_eq!(options.keyring_path(), tmp.path().join("trustedkeys.gpg"));
        assert_eq!(DEFAULT_GPG_DIR, "/etc/pacman.d/gnupg/");
        assert_eq!(XPM_GPG_DIR, "/etc/xpm/gnupg/");
    }

    #[test]
    fn test_validation_zero_parallel_downloads() {
        let toml_content = r#"
[options]
parallel_downloads = 0

[[repo]]
name = "core"
server = ["https://mirror.example.com/core"]
"#;
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        tmpfile.write_all(toml_content.as_bytes()).unwrap();

        let result = XpmConfig::load(tmpfile.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_validation_empty_repo_name() {
        let toml_content = r#"
[options]
parallel_downloads = 5

[[repo]]
name = ""
server = ["https://mirror.example.com/core"]
"#;
        let mut tmpfile = tempfile::NamedTempFile::new().unwrap();
        tmpfile.write_all(toml_content.as_bytes()).unwrap();

        let result = XpmConfig::load(tmpfile.path());
        assert!(result.is_err());
    }
}
