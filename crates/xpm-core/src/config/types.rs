//! Configuration data types and defaults.

use super::*;

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
    /// Package signature verification level override for this repo.
    pub sig_level: Option<SigLevel>,
    /// Database signature verification level override for this repo.
    pub database_sig_level: Option<SigLevel>,
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
    /// Default package signature verification level.
    pub sig_level: SigLevel,
    /// Default database signature verification level (falls back to `sig_level`).
    pub database_sig_level: Option<SigLevel>,
    /// Use color in output.
    pub color: bool,
    /// Perform operations in parallel (number of threads).
    pub parallel_downloads: u32,
    /// Check available disk space before installing.
    pub check_space: bool,
    /// Execute pacman-style ALPM hooks around transactions.
    ///
    /// Disable this when a package ships a hook that shells out to pacman
    /// (for example mkinitcpio's `pacman -Qoq` hook) and the transaction must
    /// stay fully native.
    pub alpm_hooks: bool,
    /// Create `~/.local/bin` shims and edit shell rc files for non-root roots.
    /// Disabled by default; requiring it is opt-in for development roots.
    pub shell_integration: bool,
    /// Whether package `.INSTALL` scriptlets run (auto/skip/require).
    pub scriptlet_mode: ScriptletMode,
}

impl GeneralOptions {
    /// Keyring directory actually used for signature verification.
    ///
    /// Resolution order: the configured `gpg_dir` when it exists, then the
    /// native xpm keyring, and finally the configured value (which produces
    /// the most useful error message when nothing exists yet). Pacman's
    /// keyring is only used when it is configured explicitly.
    pub fn effective_gpg_dir(&self) -> PathBuf {
        for candidate in [self.gpg_dir.as_path(), Path::new(DEFAULT_GPG_DIR)] {
            if candidate.is_dir() {
                return candidate.to_path_buf();
            }
        }
        self.gpg_dir.clone()
    }

    /// Path of the keyring file inside [`Self::effective_gpg_dir`].
    ///
    /// Prefer a legacy `trustedkeys.gpg` file or GnuPG's `pubring.gpg` inside
    /// the effective keyring directory.
    pub fn keyring_path(&self) -> PathBuf {
        let dir = self.effective_gpg_dir();
        for candidate in ["trustedkeys.gpg", "pubring.gpg"] {
            let path = dir.join(candidate);
            if path.is_file() {
                return path;
            }
        }
        dir.join("trustedkeys.gpg")
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
            database_sig_level: None,
            color: true,
            parallel_downloads: 10,
            check_space: true,
            alpm_hooks: true,
            shell_integration: false,
            scriptlet_mode: ScriptletMode::Auto,
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
                database_sig_level: None,
            }],
        }
    }
}
