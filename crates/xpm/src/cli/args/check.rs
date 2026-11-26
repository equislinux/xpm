//! `query`, `check`, `mark` and `deptest` arguments.

use std::path::PathBuf;

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

    /// Exact package name to query (substring with --search).
    pub filter: Option<String>,

    /// Treat FILTER as a substring instead of an exact package name.
    #[arg(long)]
    pub search: bool,

    /// Print only package names.
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Find the installed package that owns PATH.
    #[arg(long, value_name = "PATH")]
    pub owner: Option<String>,

    /// List packages that are not present in any configured sync database.
    #[arg(long, short = 'm')]
    pub foreign: bool,

    /// List packages that are present in a configured sync database.
    #[arg(long, short = 'n')]
    pub native: bool,

    /// Query a package file instead of the local database.
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,

    /// Show the changelog stored for an installed package.
    #[arg(long)]
    pub changelog: bool,

    /// List packages that no other installed package directly requires.
    #[arg(long)]
    pub unrequired: bool,

    /// List orphan packages (no longer required).
    #[arg(long, short = 't')]
    pub orphans: bool,

    /// Check for packages that are outdated.
    #[arg(long, short = 'u')]
    pub upgrades: bool,
}

#[derive(Debug, clap::Args)]
pub struct CheckArgs {
    /// Also verify the stored .MTREE hashes, modes and symlink targets.
    #[arg(long)]
    pub full: bool,

    /// Rebuild the `files` manifest from the stored .MTREE. This repairs
    /// corrupted ownership lists without touching installed files.
    #[arg(long, conflicts_with = "full")]
    pub repair: bool,

    /// Installed packages to check (default: all).
    pub packages: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct MarkArgs {
    /// Mark packages as dependencies.
    #[arg(long)]
    pub as_deps: bool,

    /// Mark packages as explicitly installed.
    #[arg(long)]
    pub as_explicit: bool,

    /// Installed package names to mark.
    #[arg(required = true)]
    pub packages: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct DeptestArgs {
    /// Dependency specifications to test (e.g. glibc>=2.39).
    pub dependencies: Vec<String>,
}
