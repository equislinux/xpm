//! `install` arguments.

#[derive(Debug, clap::Args)]
pub struct InstallArgs {
    /// Packages to install (`name`, `name=version`, `name>=version`, `repo/name`).
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

    /// Ignore packages in GROUP (repeatable; applied by upgrades).
    #[arg(long = "ignoregroup", value_name = "GROUP")]
    pub ignore_groups: Vec<String>,

    /// Assume package=version is installed for dependency resolution.
    #[arg(long = "assume-installed", value_name = "PKG=VER")]
    pub assume_installed: Vec<String>,
}
