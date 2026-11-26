//! Removal planning and install-time dependency checks.
//!
//! [`removal`] implements the reverse dependency checks, recursive/cascade
//! modes and reverse dependency ordering used by `pacman -R`. [`install`]
//! checks target dependencies and conflicts before a transaction.

use std::collections::{HashMap, HashSet};

use crate::error::{XpmError, XpmResult};
use crate::install_reason::InstallReason;
use crate::resolver::{DepConstraint, Operator, Version};

/// An installed package as seen by the removal planner.
#[derive(Debug, Clone)]
pub struct InstalledPkg {
    pub name: String,
    pub version: Version,
    pub depends: Vec<DepConstraint>,
    pub provides: Vec<DepConstraint>,
    pub reason: InstallReason,
}

/// Flags controlling removal planning.
#[derive(Debug, Default, Clone, Copy)]
pub struct RemoveOptions {
    /// `-Rs`: remove dependency-reason packages that become unneeded.
    pub recursive: bool,
    /// `-Rss`: also remove explicitly installed dependencies.
    pub recursive_all: bool,
    /// `-Rc`: remove packages that depend on the targets.
    pub cascade: bool,
    /// `-Ru`: keep targets that are still required.
    pub unneeded: bool,
    /// `-Rd`: ignore dependency version constraints.
    pub ignore_versions: bool,
    /// `-Rdd`: skip dependency checks entirely.
    pub skip_deps: bool,
}

mod install;
mod removal;

pub use install::{check_install, check_installed_closure, InstallTarget};
pub use removal::plan_removal;
use removal::satisfies;

#[cfg(test)]
mod install_checks;
#[cfg(test)]
mod tests;
