//! Local-database query helpers for the pacman-compatible flags.
//!
//! Implements the logic behind `pacman -Qo` (owner lookup), `-Qm` (foreign
//! packages) and `-Qt` (packages not directly required by another installed
//! package). Everything reads the xpm local database and, for `-Qm`, the
//! configured sync database names; the sync file manifests are never used.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path};

use crate::deptest::InstalledDependency;
use crate::error::XpmResult;
use crate::install_reason::InstallReason;
use crate::local_db;
use crate::resolver::DepConstraint;

mod filters;
mod local;
mod owners;

pub use filters::{find_foreign, find_unrequired};
pub use local::{load_local_packages, LocalPackage};
pub use owners::{find_owners, normalize_query_path, Owner};

#[cfg(test)]
mod tests;
