//! Pacman-compatible file conflict detection.
//!
//! This module performs the same preflight checks as
//! `_alpm_db_find_fileconflicts()` in libalpm: every target is checked
//! against every other target, then each target is checked against the
//! filesystem using the installed package manifests for ownership.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use glob::{MatchOptions, Pattern};

use crate::error::{XpmError, XpmResult};

mod check;
mod normalize;
mod overwrite;
mod owner;
mod types;

pub use check::{check_file_conflicts, check_file_conflicts_with_removals};
pub use overwrite::OverwritePatterns;
pub use types::{ConflictKind, FileConflict, InstalledManifest, TargetManifest};

use normalize::*;
use owner::*;

#[cfg(test)]
mod tests;
