//! `remove` and `upgrade` arguments.

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

    /// Download packages without committing the upgrade.
    #[arg(long, short = 'w')]
    pub download_only: bool,

    /// Overwrite conflicting files matching PATTERN (repeatable; `!` negates).
    #[arg(long, value_name = "PATTERN")]
    pub overwrite: Vec<String>,

    /// Ignore upgrades for packages in GROUP (repeatable).
    #[arg(long = "ignoregroup", value_name = "GROUP")]
    pub ignore_groups: Vec<String>,

    /// Assume package=version is installed for dependency resolution.
    #[arg(long = "assume-installed", value_name = "PKG=VER")]
    pub assume_installed: Vec<String>,
}
