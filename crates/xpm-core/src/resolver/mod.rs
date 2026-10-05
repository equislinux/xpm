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
pub use plan::{resolve_closure, Requirement};
pub use provider::XpmProvider;
pub use types::{PackageCandidate, PackageDependency, PackagePool, VersionReq};
pub use version::Version;
