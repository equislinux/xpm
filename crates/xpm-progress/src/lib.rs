//! Shared terminal progress rendering helpers.

use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

const MIB: f64 = 1024.0 * 1024.0;

mod operation;
mod parallel;
mod phase;
mod render;
mod settings;

pub use operation::OperationProgress;
pub use parallel::ParallelDownloadProgress;
pub use phase::PhaseProgress;
pub use render::{format_mib, format_progress_with, percentage, BarFill, ProgressStyle};
pub use settings::{finish_all, init, message, verbose, warn};

use render::{align_label, bar};
use settings::with_settings;

#[cfg(test)]
mod tests;
