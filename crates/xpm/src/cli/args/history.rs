//! History, rollback and diff arguments.

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
