//! Per-command clap argument structs.

mod bootstrap;
mod build;
mod check;
mod help;
mod history;
mod install;
mod listing;
mod remove;
mod repo;

pub use bootstrap::BootstrapArgs;
pub use build::BuildArgs;
pub use check::{CheckArgs, DeptestArgs, MarkArgs, QueryArgs};
pub use help::HelpArgs;
pub use history::{DiffArgs, HistoryArgs, RollbackArgs};
pub use install::InstallArgs;
pub use listing::{CleanArgs, FilesArgs, FilesDbArgs, GroupsArgs, InfoArgs, ListArgs, SearchArgs};
pub use remove::{RemoveArgs, UpgradeArgs};
pub use repo::{ImportArgs, RepoAction, RepoArgs};

// ── Subcommand arguments ────────────────────────────────────────────────────

#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    /// Force a full database refresh even if up to date.
    #[arg(long, short)]
    pub force: bool,
}
