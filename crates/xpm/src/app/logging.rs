//! Tracing/logging initialization.

use tracing::Level;
use tracing_subscriber::EnvFilter;

/// Initializes `tracing` from `-v` flags, with `RUST_LOG` taking precedence.
pub(crate) fn init(verbose: u8) {
    let log_level = match verbose {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(log_level.into())
                .from_env_lossy(),
        )
        .with_target(false)
        .init();

    tracing::debug!("xpm v{}", env!("CARGO_PKG_VERSION"));
}
