//! xpm-core — Core library for the xpm package manager.
//!
//! This crate contains the business logic, configuration management,
//! and error types shared across the xpm ecosystem.

pub mod alpm_hooks;
pub mod cache;
pub mod config;
pub mod error;
pub mod generations;
pub mod hooks;
pub mod install_reason;
pub mod journal;
pub mod local_db;
pub mod orphans;
pub mod package;
pub mod repo;
pub mod repo_db;
pub mod repo_sync;
pub mod resolver;
pub mod rollback;
pub mod signing;
pub mod transaction;
pub mod txhooks;

// Re-export key types for convenience.
pub use cache::find_package;
pub use config::XpmConfig;
pub use error::{XpmError, XpmResult};
pub use generations::{read_current as read_current_generation, GenerationPackage};
pub use hooks::{Hook, HookChain, HookContext, OperationType, PostScriptletHook, PreScriptletHook};
pub use install_reason::{retain_by_reason, InstallReason};
pub use journal::{Journal, JournalPackage};
pub use rollback::{build_plan as build_rollback_plan, RollbackOp, RollbackPlan};
pub use transaction::{FileLock, Transaction, TransactionOp, TransactionState};
pub use txhooks::{run_transaction_hooks, HookRunOutcome};
