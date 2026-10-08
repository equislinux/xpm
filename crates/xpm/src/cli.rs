use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// xpm — Modern package manager for X Distribution
///
/// A high-performance, Rust-native package manager compatible with
/// the ALPM package format and Arch Linux repositories.
#[derive(Debug, Parser)]
#[command(
    name = "xpm",
    version,
    about = "Modern package manager for X Distribution",
    long_about = "xpm is a high-performance, Rust-native package manager built for the X Distribution.\n\
                  It manages packages using the ALPM format and is compatible with Arch Linux repositories.",
    arg_required_else_help = true
)]
pub struct Cli {
    /// Path to the configuration file.
    #[arg(long, short = 'c', global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Increase output verbosity (-v, -vv, -vvv).
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Suppress confirmation prompts.
    #[arg(long, global = true)]
    pub no_confirm: bool,

    /// Set an alternative installation root.
    #[arg(long, global = true, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Set an alternative database directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub dbpath: Option<PathBuf>,

    /// Set an alternative cache directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub cachedir: Option<PathBuf>,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Synchronize package databases [-Sy].
    #[command(visible_alias = "Sy")]
    Sync(SyncArgs),

    /// Install one or more packages [-S].
    #[command(visible_alias = "S")]
    Install(InstallArgs),

    /// Remove one or more packages [-R].
    #[command(visible_alias = "R")]
    Remove(RemoveArgs),

    /// Upgrade all installed packages [-Su].
    #[command(visible_alias = "Su")]
    Upgrade(UpgradeArgs),

    /// Query the local package database [-Q].
    #[command(visible_alias = "Q")]
    Query(QueryArgs),

    /// Search for packages in the sync databases [-Ss].
    #[command(visible_alias = "Ss")]
    Search(SearchArgs),

    /// Display information about a package [-Si/-Qi].
    #[command(visible_alias = "Si")]
    Info(InfoArgs),

    /// List files owned by a package [-Ql].
    #[command(visible_alias = "Ql")]
    Files(FilesArgs),

    /// Manage repositories — add, remove, or list.
    Repo(RepoArgs),

    /// Show the transaction journal (history and rollback material).
    History(HistoryArgs),

    /// Roll back the last transaction (package level, from the cache).
    Rollback(RollbackArgs),

    /// Compare the installed packages against a generation capture.
    Diff(DiffArgs),

    /// Display detailed usage information and guides.
    Usage(HelpArgs),
}

// ── Subcommand arguments ────────────────────────────────────────────────────

#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    /// Force a full database refresh even if up to date.
    #[arg(long, short)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct InstallArgs {
    /// Package names to install.
    #[arg(required = true)]
    pub packages: Vec<String>,

    /// Only download packages, do not install.
    #[arg(long, short = 'w')]
    pub download_only: bool,

    /// Install as a dependency (not explicitly installed).
    #[arg(long)]
    pub as_deps: bool,

    /// Install as an explicit package.
    #[arg(long)]
    pub as_explicit: bool,

    /// Do not install optional dependencies.
    #[arg(long)]
    pub no_optional: bool,
}

#[derive(Debug, clap::Args)]
pub struct RemoveArgs {
    /// Package names to remove.
    #[arg(required = true)]
    pub packages: Vec<String>,

    /// Remove also unneeded dependencies (recursive).
    #[arg(long, short = 's')]
    pub recursive: bool,

    /// Skip dependency checks.
    #[arg(long, short = 'd')]
    pub no_deps: bool,

    /// Remove configuration files as well (purge).
    #[arg(long, short = 'n')]
    pub nosave: bool,
}

#[derive(Debug, clap::Args)]
pub struct UpgradeArgs {
    /// Force reinstallation of up-to-date packages.
    #[arg(long)]
    pub force: bool,

    /// Ignore specific packages during upgrade.
    #[arg(long, value_name = "PKG")]
    pub ignore: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct QueryArgs {
    /// Output format: plain (default) or tsv for machine consumption.
    #[arg(long, value_parser = ["plain", "tsv"], default_value = "plain")]
    pub format: String,

    /// List only explicitly installed packages.
    #[arg(long, short = 'e')]
    pub explicit: bool,

    /// List only packages installed as dependencies.
    #[arg(long, short = 'd')]
    pub deps: bool,

    /// Filter by package name (optional).
    pub filter: Option<String>,

    /// List orphan packages (no longer required).
    #[arg(long, short = 't')]
    pub orphans: bool,

    /// Check for packages that are outdated.
    #[arg(long, short = 'u')]
    pub upgrades: bool,
}

#[derive(Debug, clap::Args)]
pub struct SearchArgs {
    /// Search query (name, description, or provides).
    #[arg(required = true)]
    pub query: String,

    /// Search in local database instead of sync.
    #[arg(long, short)]
    pub local: bool,
}

#[derive(Debug, clap::Args)]
pub struct InfoArgs {
    /// Package name to inspect.
    #[arg(required = true)]
    pub package: String,

    /// Query local database instead of sync.
    #[arg(long, short)]
    pub local: bool,
}

#[derive(Debug, clap::Args)]
pub struct FilesArgs {
    /// Package name to list files for.
    #[arg(required = true)]
    pub package: String,
}

#[derive(Debug, clap::Args)]
pub struct RepoArgs {
    #[command(subcommand)]
    pub action: RepoAction,
}

#[derive(Debug, Subcommand)]
pub enum RepoAction {
    /// Add a temporary repository.
    Add(RepoAddArgs),

    /// Remove a temporary repository.
    Remove(RepoRemoveArgs),

    /// List all active repositories (predefined + temporary).
    List,
}

#[derive(Debug, clap::Args)]
pub struct RepoAddArgs {
    /// Repository name (e.g. "my-custom-repo").
    #[arg(required = true)]
    pub name: String,

    /// Repository URL (e.g. "https://example.com/repo/os/x86_64").
    #[arg(required = true)]
    pub url: String,
}

#[derive(Debug, clap::Args)]
pub struct RepoRemoveArgs {
    /// Name of the repository to remove.
    #[arg(required = true)]
    pub name: String,
}

#[derive(Debug, clap::Args)]
pub struct HistoryArgs {
    /// Emit one JSON object per transaction (machine consumption).
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
pub struct RollbackArgs {
    /// Undo the newest successful transaction (the default when no journal is
    /// given).
    #[arg(long)]
    pub last: bool,

    /// Journal id to undo (see `xpm history`).
    #[arg(long, value_name = "ID", conflicts_with = "last")]
    pub journal: Option<String>,

    /// Print the inverse plan without changing anything.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, clap::Args)]
pub struct DiffArgs {
    /// Generation id to compare against (`current` resolves the default one).
    #[arg(value_name = "GENERATION")]
    pub generation: String,

    /// Emit the diff as a single JSON object.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Args)]
pub struct HelpArgs {
    /// Topic to display help for (commands, config, repos, or a specific command).
    pub topic: Option<String>,
}
