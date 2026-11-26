//! TOML loading, validation and CLI overrides.

use super::*;

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
