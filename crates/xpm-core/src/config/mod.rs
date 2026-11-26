use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ConfigError;
use crate::hooks::ScriptletMode;

// ── Default paths ───────────────────────────────────────────────────────────

const DEFAULT_ROOT_DIR: &str = "/";
const DEFAULT_DB_PATH: &str = "/var/lib/xpm/";
const DEFAULT_CACHE_DIR: &str = "/var/cache/xpm/pkg/";
const DEFAULT_LOG_FILE: &str = "/var/log/xpm.log";
/// Native xpm keyring, used by default.
pub const DEFAULT_GPG_DIR: &str = "/etc/xpm/gnupg/";
/// Native xpm keyring alias kept for callers that used the old name.
pub const XPM_GPG_DIR: &str = "/etc/xpm/gnupg/";
/// Pacman keyring; only used when explicitly configured for compatibility.
pub const PACMAN_GPG_DIR: &str = "/etc/pacman.d/gnupg/";
const DEFAULT_CONFIG_PATH: &str = "/etc/xpm.conf";

// ── Configuration structs ───────────────────────────────────────────────────

mod load;
mod pacman;
mod types;

pub use types::{GeneralOptions, Repository, SigLevel, XpmConfig};

#[cfg(test)]
mod tests;
