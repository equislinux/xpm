//! xpm — Modern package manager for X Distribution
//!
//! Entry point for the xpm binary. Handles process setup, CLI parsing,
//! logging initialization, configuration loading, and dispatch into
//! [`commands`].

mod app;
mod cli;
mod commands;
mod pacman_compat;

use anyhow::Result;
use clap::Parser;

use cli::Cli;

fn main() -> Result<()> {
    // Rust ignores SIGPIPE by default, so `xpm ... | head` panics with a
    // broken-pipe error; restore the default handler like other CLIs.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let argv = pacman_compat::rewrite_args(std::env::args_os());
    let cli = Cli::parse_from(argv);

    app::logging::init(cli.verbose);
    let (config, host_gpg_dir) = app::config::load(&cli)?;

    tracing::info!(
        root = %config.options.root_dir.display(),
        db = %config.options.db_path.display(),
        repos = config.repositories.len(),
        "configuration loaded"
    );

    let result = commands::dispatch(&cli, &config, &host_gpg_dir);
    xpm_core::progress::finish_all();
    result
}
