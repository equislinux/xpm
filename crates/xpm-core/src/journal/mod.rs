//! Persistent transaction journal.
//!
//! Every transaction is recorded as a JSON file under the journal directory
//! (default: `<db_path>/journal`). The file is written before touching the
//! filesystem and finalized after the commit, so a crash leaves a `running`
//! entry with the intended operations. `xpm history` reads it back and the
//! provisioning payload (`x gen`) consumes it to align generations with xpm
//! transactions. See `docs/GENERATIONS.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{XpmError, XpmResult};

pub const JOURNAL_SCHEMA: u32 = 1;

mod entry;
mod recovery;
mod store;
mod time;

pub use entry::JournalPackage;
pub use recovery::{reconcile_running, RecoveryReport};
pub use store::Journal;
pub use time::{iso8601, now_epoch};

#[cfg(test)]
mod tests;
