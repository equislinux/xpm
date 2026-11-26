//! Long-form usage help.

use super::*;

mod command;
mod topics;

use command::print_help_command;
use topics::{print_help_commands, print_help_config, print_help_overview, print_help_repos};

pub(super) fn cmd_help(args: &cli::HelpArgs) -> Result<()> {
    match args.topic.as_deref() {
        None | Some("") => print_help_overview(),
        Some("commands") => print_help_commands(),
        Some("config") => print_help_config(),
        Some("repos") | Some("repositories") => print_help_repos(),
        Some(cmd) => print_help_command(cmd),
    }
    Ok(())
}
