//! Repository and pacman-import arguments.

use std::path::PathBuf;

use clap::Subcommand;

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
pub struct ImportArgs {
    /// pacman local database directory.
    #[arg(long, value_name = "PATH", default_value = "/var/lib/pacman/local")]
    pub from: PathBuf,

    /// Overwrite entries that already exist in the xpm database.
    #[arg(long)]
    pub force: bool,
}
