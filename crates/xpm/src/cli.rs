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
    #[arg(long, short = 'r', global = true, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Set an alternative database directory.
    #[arg(long, short = 'b', global = true, value_name = "PATH")]
    pub dbpath: Option<PathBuf>,

    /// Set an alternative cache directory.
    #[arg(long, global = true, value_name = "PATH")]
    pub cachedir: Option<PathBuf>,

    /// Disable colored output.
    #[arg(long, global = true)]
    pub no_color: bool,

    /// Do not mirror transactions into pacman's local database.
    #[arg(long, global = true)]
    pub no_pacman_mirror: bool,

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

    /// Rewrite pacman's local database from xpm's database.
    #[command(name = "sync-pacman-db")]
    SyncPacmanDb(SyncPacmanDbArgs),

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

    /// Skip packages that are already installed at the same version.
    #[arg(long)]
    pub needed: bool,

    /// Print the resolved packages without downloading or installing them.
    #[arg(long, short = 'p')]
    pub print: bool,

    /// Overwrite conflicting files matching PATTERN (repeatable; `!` negates).
    #[arg(long, value_name = "PATTERN")]
    pub overwrite: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct BuildArgs {
    /// Directory containing PKGBUILD (or the PKGBUILD file itself).
    #[arg(value_name = "DIR", default_value = ".")]
    pub path: PathBuf,

    /// Directory for built packages (default: <DIR>/.xpm-packages).
    #[arg(long, value_name = "PATH")]
    pub pkgdest: Option<PathBuf>,

    /// makepkg source cache directory.
    #[arg(long, value_name = "PATH")]
    pub srcdest: Option<PathBuf>,

    /// makepkg build directory.
    #[arg(long, value_name = "PATH")]
    pub builddir: Option<PathBuf>,

    /// Skip the check() function.
    #[arg(long)]
    pub nocheck: bool,

    /// Remove existing package files from --pkgdest before building.
    #[arg(long)]
    pub clean: bool,

    /// Build inside a clean devtools chroot (requires devtools and sudo).
    #[arg(long)]
    pub chroot: bool,

    /// Chroot root/copy directory (default: /var/lib/archbuild/xpm-aur).
    #[arg(long, value_name = "PATH")]
    pub chroot_dir: Option<PathBuf>,

    /// Install the built packages with xpm after a successful build.
    #[arg(long)]
    pub install: bool,

    /// Mark installed packages as dependencies.
    #[arg(long)]
    pub as_deps: bool,

    /// Mark installed packages as explicit.
    #[arg(long)]
    pub as_explicit: bool,

    /// Install a local package into the clean chroot before building.
    #[arg(long, value_name = "PKG")]
    pub preinstall: Vec<PathBuf>,

    /// Write the build result as JSON to this path.
    #[arg(long, value_name = "PATH")]
    pub json: Option<PathBuf>,

    /// Extra arguments passed through to makepkg after `--`.
    #[arg(last = true, allow_hyphen_values = true)]
    pub makepkg_args: Vec<String>,
}

#[derive(Debug, clap::Args)]
pub struct RemoveArgs {
    /// Package names to remove.
    #[arg(required = true)]
    pub packages: Vec<String>,

    /// Remove unneeded dependencies; repeat (-Rss) to include explicit ones.
    #[arg(long, short = 's', action = clap::ArgAction::Count)]
    pub recursive: u8,

    /// Ignore dependency versions (-d) or all dependency checks (-dd).
    #[arg(long = "nodeps", alias = "no-deps", short = 'd', action = clap::ArgAction::Count)]
    pub no_deps: u8,

    /// Remove packages that depend on the targets (pacman -Rc).
    #[arg(long)]
    pub cascade: bool,

    /// Keep targets that are still required by another package (-Ru).
    #[arg(long, short = 'u')]
    pub unneeded: bool,

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

    /// Print the upgrade plan without downloading or installing anything.
    #[arg(long, short = 'p')]
    pub print: bool,

    /// Overwrite conflicting files matching PATTERN (repeatable; `!` negates).
    #[arg(long, value_name = "PATTERN")]
    pub overwrite: Vec<String>,
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
pub struct GroupsArgs {
    /// List local groups instead of sync repository groups.
    #[arg(long)]
    pub local: bool,

    /// Print only package names.
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Restrict output to one group.
    pub group: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ImportArgs {
    /// pacman local database directory.
    #[arg(long, value_name = "PATH", default_value = "/var/lib/pacman/local")]
    pub from: PathBuf,

    /// Overwrite entries that already exist in the xpm database.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct SyncPacmanDbArgs {
    /// pacman database directory (default: configured or auto-detected).
    #[arg(long, value_name = "PATH")]
    pub pacman_db: Option<PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct HelpArgs {
    /// Topic to display help for (commands, config, repos, or a specific command).
    pub topic: Option<String>,
}
