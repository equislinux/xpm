//! Reader for pacman 7.1.0 `pacman.conf` files.
//!
//! The parser mirrors `src/pacman/ini.c` and `src/pacman/conf.c`: line-oriented
//! INI, textual `Include` expansion with shared section state, raw `SigLevel`
//! bitfields, and pacman's option defaults. Paths are never resolved or
//! canonicalized; `$repo` and `$arch` are left untouched for the caller.

use std::fmt;
use std::fs;
use std::ops::{BitOr, BitOrAssign};
use std::path::{Path, PathBuf};

use crate::error::{ConfigError, XpmError, XpmResult};

const MAX_INCLUDE_DEPTH: usize = 10;

const DEFAULT_ROOT_DIR: &str = "/";
const DEFAULT_DB_PATH: &str = "/var/lib/pacman/";
const DEFAULT_CACHE_DIR: &str = "/var/cache/pacman/pkg/";
const DEFAULT_LOG_FILE: &str = "/var/log/pacman.log";
const DEFAULT_GPG_DIR: &str = "/etc/pacman.d/gnupg/";
const DEFAULT_HOOK_DIR: &str = "/etc/pacman.d/hooks/";

mod builders;
mod include;
mod parser;
mod siglevel;
mod types;

pub use parser::parse_pacman_conf;
pub use types::{
    CleanMethod, ColorMode, PacmanConfig, PacmanOptions, PacmanRepo, SigCheck, SigLevelState,
    SigTrust, SourceLoc, Usage,
};

use builders::{OptionsBuilder, RepoBuilder};
use include::expand_include;
use parser::under_root;
use siglevel::{parse_parallel_downloads, parse_sig_level, parse_usage};

fn config_error(file: &Path, line: usize, message: impl fmt::Display) -> XpmError {
    XpmError::Config(ConfigError::Validation {
        message: format!("config file {}, line {}: {}", file.display(), line, message),
    })
}

fn split_values(value: &str) -> impl Iterator<Item = &str> {
    value.split(' ').filter(|token| !token.is_empty())
}

#[cfg(test)]
mod tests;
