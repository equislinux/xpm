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

    /// Suppress confirmation prompts (pacman-compatible alias: --noconfirm).
    #[arg(long, global = true, alias = "noconfirm")]
    pub no_confirm: bool,

    /// Disable per-phase progress rendering (pacman/makepkg compatibility).
    #[arg(long, global = true, hide = true)]
    pub noprogressbar: bool,

    /// Set an alternative installation root.
    #[arg(long, short = 'r', global = true, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Operate on a mounted guest system (root, database and cache paths).
    #[arg(long, global = true, value_name = "PATH")]
    pub sysroot: Option<PathBuf>,

    /// Set an alternative database directory.
    #[arg(long, short = 'b', global = true, value_name = "PATH")]
    pub dbpath: Option<PathBuf>,

    /// Set an alternative cache directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub cachedir: Option<PathBuf>,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Deprecated no-op: xpm never writes pacman's local database.
    #[arg(long, global = true, hide = true)]
    pub no_pacman_mirror: bool,

    /// Do not execute pacman-style ALPM hooks for this transaction.
    #[arg(long, global = true)]
    pub no_alpm_hooks: bool,

    /// Enable ~/.local/bin shims and rc edits for non-root installs.
    #[arg(long, global = true)]
    pub shell_integration: bool,

    /// Disable shell integration (default; accepted for compatibility).
    #[arg(long, global = true)]
    pub no_shell_integration: bool,

    /// Scriptlet policy: auto, skip, or require.
    #[arg(long, global = true, value_name = "MODE", value_parser = ["auto", "skip", "require"])]
    pub scriptlets: Option<String>,

    /// Skip package .INSTALL scriptlets (alias for --scriptlets skip).
    #[arg(long, global = true)]
    pub noscriptlet: bool,

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

    /// Build a source package with makepkg and collect its artifacts.
    Build(BuildArgs),

    /// Bootstrap an empty root with xpm, without pacman or pacstrap.
    #[command(name = "bootstrap")]
    Bootstrap(BootstrapArgs),

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

    /// Import installed packages from pacman's local database.
    #[command(name = "import-pacman")]
    ImportPacman(ImportArgs),

    /// Verify installed files against the local database [-Qk/-Qkk].
    #[command(visible_alias = "Qk")]
    Check(CheckArgs),

    /// Change the install reason of installed packages [-D].
    #[command(visible_alias = "D")]
    Mark(MarkArgs),

    /// Test dependency specifications against installed packages [-T].
    #[command(visible_alias = "T")]
    Deptest(DeptestArgs),

    /// List package groups (sync or local).
    Groups(GroupsArgs),

    /// List packages available in the sync repositories [-Sl].
    List(ListArgs),

    /// Remove cached package files [-Sc/-Scc].
    Clean(CleanArgs),

    /// Query the sync files databases [-F].
    #[command(name = "file-db")]
    FileDb(FilesDbArgs),

    /// Display detailed usage information and guides.
    Usage(HelpArgs),
}

mod args;

pub use args::*;
