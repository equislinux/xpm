//! High-level resolution: requirements in, dependency-ordered plan out.
//!
//! This is the entry point the CLI (`xpm install`) uses to turn a list of
//! requested package names into a concrete install plan. It builds the
//! [`PackagePool`], feeds the [`XpmProvider`] SAT solver, and returns the
//! solved candidates in dependency order (dependencies before dependents).

use std::collections::{HashMap, HashSet};

use resolvo::{ArenaId, NameId, Problem, SolvableId, Solver, VersionSetId};

use crate::error::{XpmError, XpmResult};
use crate::resolver::dependency::{DepConstraint, Operator};
use crate::resolver::provider::XpmProvider;
use crate::resolver::types::{PackageCandidate, PackagePool};
use crate::resolver::version::Version;

/// A requested package: a name plus an optional ALPM version constraint and
/// optional repository qualifier (`repo/name`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// Requested package name (may be a virtual name provided by a package).
    pub name: String,
    /// Comparison operator when the request carried a version constraint.
    pub op: Option<Operator>,
    /// Version when the request carried a constraint.
    pub version: Option<Version>,
    /// Repository name when the request used `repo/name`.
    pub repo: Option<String>,
}

impl Requirement {
    /// Parse a CLI requirement using ALPM dependency syntax:
    ///
    /// - `foo` means any version from any repository.
    /// - `foo=1.2-1`, `foo>=1.2`, `foo>1.2`, `foo<=1.2` and `foo<1.2` are
    ///   accepted; the same operators are used for local package dependencies.
    /// - `extra/foo` restricts the request to a repository.
    /// - `extra/foo>=1.2` combines both.
    pub fn parse(spec: &str) -> XpmResult<Self> {
        let (repo, target) = match spec.split_once('/') {
            Some((repo, name)) => {
                if repo.is_empty() || name.is_empty() || name.contains('/') {
                    return Err(XpmError::Package(format!(
                        "invalid package requirement '{spec}' (expected repo/name)"
                    )));
                }
                (Some(repo.to_string()), name)
            }
            None => (None, spec),
        };

        if target.is_empty() {
            return Err(XpmError::Package("empty package name".to_string()));
        }

        let dependency = DepConstraint::parse(target);
        if dependency.name.is_empty() {
            return Err(XpmError::Package(format!(
                "invalid package requirement '{spec}' (empty package name)"
            )));
        }
        if dependency.op.is_some()
            && dependency
                .version
                .as_ref()
                .is_none_or(|version| version.to_string().is_empty())
        {
            return Err(XpmError::Package(format!(
                "invalid package requirement '{spec}' (missing version after operator)"
            )));
        }

        Ok(Self {
            name: dependency.name,
            op: dependency.op,
            version: dependency.version,
            repo,
        })
    }

    fn constraint(&self) -> DepConstraint {
        DepConstraint {
            name: self.name.clone(),
            op: self.op,
            version: self.version.clone(),
        }
    }

    /// Human-readable requirement, including repository and version.
    pub fn display(&self) -> String {
        let base = match &self.repo {
            Some(repo) => format!("{repo}/{}", self.name),
            None => self.name.clone(),
        };
        match (&self.op, &self.version) {
            (Some(op), Some(version)) => format!("{base}{op}{version}"),
            _ => base,
        }
    }
}

mod closure;
mod conflicts;
mod sat;

pub use closure::{resolve_closure, resolve_closure_with_installed};

use closure::package_satisfies;
use conflicts::{check_solution_conflicts, conflict_with_installed};
use sat::{find_version_set, intern_once};

#[cfg(test)]
mod tests;
