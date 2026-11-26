//! `usage` argument.

#[derive(Debug, clap::Args)]
pub struct HelpArgs {
    /// Topic to display help for (commands, config, repos, or a specific command).
    pub topic: Option<String>,
}
