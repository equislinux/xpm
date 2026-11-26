//! Sysupgrade replacement planning from `replaces` metadata.
//!
//! pacman consults `replaces` only during sysupgrade (`-Su`/`-Suu`), never for
//! a plain install of a new package. This module performs selection only: the
//! caller must add each old package to the transaction remove set and run
//! dependency, conflict, and file planning against `(local - remove) + add`.
//! `replaces` does not imply `conflicts` or `provides`.

use glob::{MatchOptions, Pattern};

use crate::install_reason::InstallReason;
use crate::resolver::{DepConstraint, Version};

mod patterns;
mod resolution;

pub use patterns::matches_ignore_group;
pub use resolution::{find_replacements, InstalledReplacement, Replacement, ReplacementCandidate};

use patterns::{is_ignored, IgnorePatterns};

#[cfg(test)]
mod tests;
