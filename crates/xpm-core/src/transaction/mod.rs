//! Transaction engine for install, remove, and upgrade operations.
//!
//! This module provides the core transaction logic: planning dependency
//! resolution, preparing file extractions and removals, committing changes
//! to the filesystem, and rolling back on failure.
//!
//! Transactions are atomic where possible and logged for auditing.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::error::{XpmError, XpmResult};
use crate::hooks::{HookChain, HookContext, OperationType, ScriptletMode};
use crate::install_reason::InstallReason;
use crate::package::{PackageMeta, PackageMetadata};
use crate::progress::{self, BarFill, OperationProgress};

mod accessors;
mod commit;
mod execute;
mod fast_install;
mod lock;
mod op;
mod plan;
mod prepare;

pub use lock::FileLock;
pub use op::{TransactionOp, TransactionState};

use fast_install::{
    extraction_workers, print_operation_progress, run_fast_install, run_fast_install_parts,
};

/// A package management transaction with plan, prepare, and commit phases.
pub struct Transaction {
    operations: Vec<TransactionOp>,
    state: TransactionState,
    lock: Option<FileLock>,
    log_file: PathBuf,
    root_dir: PathBuf,
    local_db_dir: PathBuf,
    rollback_counter: usize,
    hooks_chain: Option<HookChain>,
    shell_integration: bool,
    save_configs: bool,
    scriptlet_mode: ScriptletMode,
    metadata_cache: std::collections::HashMap<PathBuf, Arc<PackageMetadata>>,
    transaction_paths: Option<Arc<std::collections::HashSet<String>>>,
}

impl Transaction {
    /// Create a new transaction with default root and database directories.
    pub fn new(root_dir: PathBuf, local_db_dir: PathBuf) -> XpmResult<Self> {
        let lock_path = local_db_dir
            .parent()
            .unwrap_or(local_db_dir.as_path())
            .join("transaction.lock");
        let log_file = root_dir.join("var/log/xpm.log");

        let lock = Some(FileLock::acquire(&lock_path)?);

        fs::create_dir_all(&local_db_dir)?;
        fs::create_dir_all(log_file.parent().unwrap_or_else(|| Path::new("/")))?;

        Ok(Transaction {
            operations: Vec::new(),
            state: TransactionState::Planning,
            lock,
            log_file,
            root_dir,
            local_db_dir,
            rollback_counter: 0,
            hooks_chain: None,
            shell_integration: false,
            save_configs: true,
            scriptlet_mode: ScriptletMode::Auto,
            metadata_cache: std::collections::HashMap::new(),
            transaction_paths: None,
        })
    }

    /// Set the hooks chain to be executed during commit.
    pub fn set_hooks(&mut self, hooks_chain: HookChain) {
        self.hooks_chain = Some(hooks_chain);
    }

    /// Enable/disable shell integration for non-root installs.
    pub fn set_shell_integration(&mut self, enabled: bool) {
        self.shell_integration = enabled;
    }

    /// Keep modified configuration files on remove/upgrade (default `true`;
    /// `false` is `--nosave`).
    pub fn set_save_configs(&mut self, enabled: bool) {
        self.save_configs = enabled;
    }

    pub fn set_scriptlet_mode(&mut self, mode: ScriptletMode) {
        self.scriptlet_mode = mode;
    }

    /// Register the file paths shipped by every target in this transaction.
    ///
    /// Upgrade cleanup uses this to avoid deleting a path that another target
    /// is taking over, regardless of commit order.
    pub fn set_transaction_paths(&mut self, paths: Arc<std::collections::HashSet<String>>) {
        self.transaction_paths = Some(paths);
    }

    /// Cache the metadata parsed while building the install plan. The hooks
    /// reuse it so each package archive is decompressed once for metadata and
    /// once for extraction instead of once per metadata consumer.
    pub fn cache_package_metadata(&mut self, pkg_file: PathBuf, metadata: Arc<PackageMetadata>) {
        self.metadata_cache.insert(pkg_file, metadata);
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        let _ = self.lock.take().map(|mut l| l.release());
    }
}

#[cfg(test)]
mod tests;
