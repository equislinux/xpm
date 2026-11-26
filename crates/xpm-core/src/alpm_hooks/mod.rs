//! ALPM hook files (`/usr/share/libalpm/hooks/*.hook` and
//! `/etc/pacman.d/hooks/*.hook`).
//!
//! X Linux ships pacman hooks that snapshot generations around transactions
//! (`10-x-gen-pre.hook` / `20-x-gen-post.hook`). Running the same `.hook`
//! files keeps xpm transactions generation-aware without any xpm→x-scripts
//! dependency.
//!
//! Supported syntax (the subset needed in practice):
//!
//! ```ini
//! [Trigger]
//! Operation = Install        # Install | Upgrade | Remove (repeatable)
//! Type = Package             # Package | Path
//! Target = *                 # glob for Package names; prefix for Path
//!
//! [Action]
//! Description = ...
//! When = PreTransaction      # PreTransaction | PostTransaction (both allowed)
//! Exec = /usr/share/x/hooks/pacman-gen.sh pre
//! Depends = x-scripts        # skip the hook when not installed
//! AbortOnFail
//! NeedsTargets               # feed the affected package names on stdin
//! ```
//!
//! Multiple `[Trigger]` sections are OR-ed; targets and operations inside one
//! trigger are AND-ed. Hook order is the lexical file-name order, matching
//! pacman and making `10-`/`20-` prefixes meaningful. `Path` triggers are
//! parsed but only matched when the caller provides the transaction's file
//! paths (xpm package operations do not always expose them); `Package`
//! triggers cover the generation hooks.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{XpmError, XpmResult};

/// Environment variable overriding the hook directories (colon-separated),
/// used by tests and by non-standard installations.
pub const ENV_HOOK_DIRS: &str = "XPM_ALPM_HOOKS_DIRS";

/// Set to `0` to stop xpm from putting its pacman query shim on the hook PATH.
pub const ENV_PACMAN_SHIM: &str = "XPM_PACMAN_SHIM";

/// Default hook directories, in priority order (later wins by file name).
pub const DEFAULT_HOOK_DIRS: &[&str] = &["/usr/share/libalpm/hooks", "/etc/pacman.d/hooks"];

mod exec;
mod parse;
mod query;
mod types;

pub use exec::{enforce_failures, run_hooks};
pub use parse::{hook_dirs, hook_dirs_for, load_hooks, parse_hook};
pub use query::{glob_match, installed_names};
pub use types::{
    AlpmHook, HookFailure, HookOperation, HookTransaction, HookWhen, Trigger, TriggerType,
};

#[cfg(test)]
use exec::{split_exec, PacmanShim};

#[cfg(test)]
mod tests;
