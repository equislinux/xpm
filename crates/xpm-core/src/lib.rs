//! xpm-core — Core library for the xpm package manager.
//!
//! This crate contains the business logic, configuration management,
//! and error types shared across the xpm ecosystem.

pub mod config;
pub mod error;
pub mod hooks;
pub mod install_reason;
pub mod journal;
pub mod local_db;
pub mod package;
pub mod repo;
pub mod repo_db;
pub mod repo_sync;
pub mod resolver;
pub mod signing;
pub mod transaction;
pub mod txhooks;

// Re-export key types for convenience.
pub use config::XpmConfig;
pub use error::{XpmError, XpmResult};
pub use hooks::{Hook, HookChain, HookContext, OperationType, PostScriptletHook, PreScriptletHook};
pub use install_reason::{retain_by_reason, InstallReason};
pub use journal::{Journal, JournalPackage};
pub use transaction::{FileLock, Transaction, TransactionOp, TransactionState};
pub use txhooks::{run_transaction_hooks, HookRunOutcome};
