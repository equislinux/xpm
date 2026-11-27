//! Transaction operation and state types.

use super::*;
use crate::install_reason::InstallReason;

/// Single operation within a transaction.
#[derive(Debug, Clone)]
pub enum TransactionOp {
    /// Install a package from a .xp file.
    Install {
        pkg_name: String,
        pkg_version: String,
        pkg_file: PathBuf,
        metadata: Box<Option<PackageMeta>>,
        reason: InstallReason,
        origin: Option<String>,
    },
    /// Remove an installed package.
    Remove { pkg_name: String },
    /// Upgrade from one version to another.
    Upgrade {
        pkg_name: String,
        old_version: String,
        new_version: String,
        new_pkg_file: PathBuf,
        reason: InstallReason,
        origin: Option<String>,
    },
}

/// State of a transaction during its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    /// Operations collected but not yet prepared.
    Planning,
    /// Pre-flight checks passed, ready to commit.
    Prepared,
    /// Changes written to filesystem and database.
    Committed,
    /// Rolled back due to error or user request.
    RolledBack,
}
