//! Repository database parsing for ALPM-style `.db` and `.files` archives.
//!
//! This module provides the foundation for sync database support (roadmap
//! phase 5), including parsing package metadata and file listings.

use std::collections::HashMap;
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

use flate2::read::GzDecoder;
use xz2::read::XzDecoder;

use crate::XpmError;
use crate::XpmResult;

mod files_db;
mod parse;
mod query;
mod sync_db;
mod types;

pub use files_db::merge_files_db;
pub use query::matches_query;
pub use sync_db::{parse_sync_db, parse_sync_db_cached};
pub use types::{LocalDb, RepoEntry, SyncDb};

pub(crate) use parse::{first_value, parse_alpm_sections};
use parse::{merge_sections, read_repo_archive, repo_entry_from_sections};

#[cfg(test)]
use files_db::merge_files_db_bytes;
#[cfg(test)]
use sync_db::parse_sync_db_bytes;

#[cfg(test)]
mod tests;
