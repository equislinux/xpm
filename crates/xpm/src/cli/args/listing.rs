//! Search, info, file-list, group, list, clean and file-db arguments.

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
pub struct GroupsArgs {
    /// List local groups instead of sync repository groups.
    #[arg(long)]
    pub local: bool,

    /// Print only package names.
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// With no group argument, print group names instead of members.
    #[arg(long)]
    pub names: bool,

    /// Restrict output to one group.
    pub group: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ListArgs {
    /// Restrict the listing to one repository.
    pub repo: Option<String>,

    /// Print only package names.
    #[arg(long, short = 'q')]
    pub quiet: bool,
}

#[derive(Debug, clap::Args)]
pub struct CleanArgs {
    /// Remove every cached package file, including the installed versions.
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, clap::Args)]
pub struct FilesDbArgs {
    /// Refresh the sync databases (including .files) before querying.
    #[arg(long, short = 'y', action = clap::ArgAction::Count)]
    pub refresh: u8,

    /// List files owned by the target package names instead of searching paths.
    #[arg(long, short = 'l')]
    pub list: bool,

    /// Print only repository/package identifiers.
    #[arg(long, short = 'q')]
    pub quiet: bool,

    /// Restrict results to one repository.
    #[arg(long, value_name = "REPO")]
    pub repo: Option<String>,

    /// File paths (default) or package names with --list.
    pub targets: Vec<String>,
}
