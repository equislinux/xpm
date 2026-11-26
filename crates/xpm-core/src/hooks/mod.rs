//! Pre/post operation hooks for transactions.
//!
//! Hooks provide extensible points to execute arbitrary logic before and
//! after install, remove, and upgrade operations. Built-in hooks include
//! local database registration and file removal.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar;
use xz2::read::XzDecoder;
use zstd::Decoder;

use crate::error::{XpmError, XpmResult};

/// Extension appended when a modified configuration file must be preserved.
const PACNEW_SUFFIX: &str = ".pacnew";
/// Extension appended when a modified configuration file is left behind by a
/// removal or by an upgrade that drops it (`--nosave` disables this).
const PACSAVE_SUFFIX: &str = ".pacsave";

/// A hook that executes before or after an operation.
pub trait Hook: Send + Sync {
    fn name(&self) -> &str;
    fn run(&self, context: &HookContext) -> XpmResult<()>;
}

/// Context passed to hooks during execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptletMode {
    /// Run when the target shell exists; skip with a warning otherwise.
    #[default]
    Auto,
    /// Never run package scriptlets.
    Skip,
    /// Fail the transaction if a scriptlet cannot run or exits nonzero.
    Require,
}

#[derive(Clone, Debug)]
pub struct HookContext {
    pub operation_type: OperationType,
    pub pkg_name: String,
    pub pkg_version: String,
    /// Version being replaced (upgrades only). Lets config-file handling find
    /// the previous manifest without guessing.
    pub old_version: Option<String>,
    pub pkg_file: Option<PathBuf>,
    pub root_dir: PathBuf,
    pub local_db_dir: PathBuf,
    /// Keep modified configuration files on remove/upgrade (`--nosave` disables).
    pub save_configs: bool,
    pub shell_integration: bool,
    pub scriptlet_mode: ScriptletMode,
    /// Metadata parsed once by the install/upgrade loop. When present the
    /// hooks avoid reopening the package archive just to read .PKGINFO,
    /// .MTREE or .INSTALL.
    pub metadata: Option<Arc<crate::package::PackageMetadata>>,
    /// File paths shipped by every target in the transaction. Upgrade cleanup
    /// must not delete a file that another target is taking over (for example
    /// a library moving from `openssl` to `openssl-3`).
    pub transaction_paths: Option<Arc<HashSet<String>>>,
}

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
pub enum OperationType {
    Install,
    Remove,
    Upgrade,
}

mod extraction;
mod fsutil;
mod local_db;
mod metadata;
mod removal;
mod scriptlet;

pub use extraction::FileExtractionHook;
pub use local_db::LocalDbHook;
pub use metadata::MetadataLoadHook;
pub use removal::FileRemovalHook;
pub use scriptlet::{PostScriptletHook, PreScriptletHook};

use crate::shell::shell_quote;
use extraction::{append_suffix, load_installed_hashes};
use fsutil::*;
use removal::{prune_empty_ancestors, remove_tracked_path};
use scriptlet::{ensure_shell_path_on_bash_zsh, ensure_shell_shim};

/// Hook chain executor — runs multiple hooks in sequence.
pub struct HookChain {
    hooks: Vec<Box<dyn Hook>>,
}

impl HookChain {
    pub fn new() -> Self {
        HookChain { hooks: Vec::new() }
    }

    pub fn add_hook(&mut self, hook: Box<dyn Hook>) {
        self.hooks.push(hook);
    }

    pub fn run(&self, context: &HookContext) -> XpmResult<()> {
        for hook in &self.hooks {
            let result = hook.run(context);
            if let Err(e) = result {
                return Err(XpmError::Package(format!(
                    "hook '{}' failed: {}",
                    hook.name(),
                    e
                )));
            }
        }
        Ok(())
    }

    pub fn hooks(&self) -> &[Box<dyn Hook>] {
        &self.hooks
    }
}

impl Default for HookChain {
    fn default() -> Self {
        let mut chain = HookChain::new();
        // Hook order: pre-scriptlet → file extraction → file removal → post-scriptlet → local db
        chain.add_hook(Box::new(MetadataLoadHook));
        chain.add_hook(Box::new(PreScriptletHook));
        chain.add_hook(Box::new(FileExtractionHook));
        chain.add_hook(Box::new(FileRemovalHook));
        chain.add_hook(Box::new(PostScriptletHook));
        chain.add_hook(Box::new(LocalDbHook));
        chain
    }
}

#[cfg(test)]
mod tests;
