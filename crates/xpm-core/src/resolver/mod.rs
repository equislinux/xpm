//! Dependency resolution engine for xpm.
//!
//! This module implements a SAT-based dependency resolver using the `resolvo`
//! crate. It provides:
//!
//! - ALPM-compatible version parsing and comparison (`vercmp`)
//! - Dependency string parsing (`>=`, `<=`, `=`, `>`, `<`)
//! - Package pool management and interning
//! - A [`DependencyProvider`] implementation that bridges xpm's package model
//!   to resolvo's solver

mod dependency;
mod plan;
mod provider;
mod types;
mod version;

#[cfg(test)]
mod tests;

pub use dependency::{DepConstraint, Operator};
pub use plan::{resolve_closure, resolve_closure_with_installed, Requirement};
pub use provider::XpmProvider;
pub use types::{PackageCandidate, PackageDependency, PackagePool, VersionReq};
pub use version::Version;

use crate::error::{XpmError, XpmResult};

/// Parses `--assume-installed package=version` specifications into synthetic
/// installed candidates.
///
/// The returned candidates have no dependencies or provides; they only make
/// the named package/version available to dependency resolution and checks.
pub fn assume_installed_candidates(specs: &[String]) -> XpmResult<Vec<PackageCandidate>> {
    let mut candidates = Vec::with_capacity(specs.len());

    for spec in specs {
        let Some((name, version)) = spec.split_once('=') else {
            return Err(XpmError::Package(format!(
                "invalid --assume-installed '{spec}' (expected package=version)"
            )));
        };
        if name.is_empty() || version.is_empty() {
            return Err(XpmError::Package(format!(
                "invalid --assume-installed '{spec}' (expected package=version)"
            )));
        }

        candidates.push(PackageCandidate {
            name: name.to_string(),
            version: Version::parse(version),
            repo: None,
            depends: Vec::new(),
            conflicts: Vec::new(),
            provides: Vec::new(),
            replaces: Vec::new(),
            optdepends: Vec::new(),
        });
    }

    Ok(candidates)
}
