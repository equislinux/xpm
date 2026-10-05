//! Core types for the dependency resolver.
//!
//! Provides the package pool that interns names, version sets, and solvables,
//! and the bridge types between xpm's package model and resolvo's ID-based system.

use std::collections::HashMap;

use resolvo::{ArenaId, NameId, SolvableId, StringId, VersionSetId, VersionSetUnionId};

use crate::resolver::dependency::DepConstraint;
use crate::resolver::version::Version;

// ── Package candidate ────────────────────────────────────────────────────────

/// A single package version that the resolver can consider installing.
#[derive(Debug, Clone)]
pub struct PackageCandidate {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: Version,
    /// Run-time dependencies (ALPM `depend` entries).
    pub depends: Vec<DepConstraint>,
    /// Packages that conflict with this one.
    pub conflicts: Vec<DepConstraint>,
    /// Virtual packages provided by this candidate (ALPM `provides`).
    pub provides: Vec<DepConstraint>,
    /// Optional dependencies.
    pub optdepends: Vec<String>,
}

/// Convenience alias for a dependency used in resolver output.
pub type PackageDependency = DepConstraint;

/// A version requirement (used for display).
pub type VersionReq = DepConstraint;

// ── Version set entry ────────────────────────────────────────────────────────

/// Associates a version set ID with the constraint it represents and the
/// package name it applies to.
#[derive(Debug, Clone)]
pub(crate) struct VersionSetEntry {
    pub name_id: NameId,
    pub constraint: DepConstraint,
    /// When true, the constraint match is inverted. This is used for
    /// conflict version sets: resolvo forbids candidates that do NOT match
    /// the version set, so we negate the constraint so that the candidates
    /// we want to forbid appear as non-matching.
    pub negated: bool,
}

/// Associates a version set union ID with its member version sets.
#[derive(Debug, Clone)]
pub(crate) struct VersionSetUnionEntry {
    pub members: Vec<VersionSetId>,
}

// ── Package pool ─────────────────────────────────────────────────────────────

/// Interning pool that maps xpm's package model to resolvo's ID-based system.
///
/// This is the central data structure used by [`super::XpmProvider`]. It manages
/// the mapping between human-readable names/versions and the opaque IDs that
/// resolvo uses internally.
#[derive(Debug)]
pub struct PackagePool {
    // ── Names ────────────────────────────────────────────────
    /// name string → NameId
    pub(crate) name_to_id: HashMap<String, NameId>,
    /// NameId → name string
    pub(crate) id_to_name: Vec<String>,

    // ── Solvables ────────────────────────────────────────────
    /// SolvableId → candidate info
    pub(crate) solvables: Vec<PackageCandidate>,
    /// package name → list of solvable ids
    pub(crate) name_to_solvables: HashMap<NameId, Vec<SolvableId>>,

    // ── Version sets ─────────────────────────────────────────
    /// VersionSetId → entry
    pub(crate) version_sets: Vec<VersionSetEntry>,

    // ── Version set unions ───────────────────────────────────
    /// VersionSetUnionId → entry
    pub(crate) version_set_unions: Vec<VersionSetUnionEntry>,

    // ── Strings ──────────────────────────────────────────────
    /// StringId → string
    pub(crate) strings: Vec<String>,
    /// string → StringId
    pub(crate) string_to_id: HashMap<String, StringId>,
}

impl PackagePool {
    /// Create a new empty package pool.
    pub fn new() -> Self {
        Self {
            name_to_id: HashMap::new(),
            id_to_name: Vec::new(),
            solvables: Vec::new(),
            name_to_solvables: HashMap::new(),
            version_sets: Vec::new(),
            version_set_unions: Vec::new(),
            strings: Vec::new(),
            string_to_id: HashMap::new(),
        }
    }

    // ── Name interning ───────────────────────────────────────────────

    /// Intern a package name, returning its ID. Returns existing ID if
    /// already interned.
    pub fn intern_name(&mut self, name: &str) -> NameId {
        if let Some(&id) = self.name_to_id.get(name) {
            return id;
        }
        let id = NameId::from_usize(self.id_to_name.len());
        self.id_to_name.push(name.to_string());
        self.name_to_id.insert(name.to_string(), id);
        id
    }

    /// Get the name string for a NameId.
    pub fn name_str(&self, id: NameId) -> &str {
        &self.id_to_name[id.to_usize()]
    }

    // ── String interning ─────────────────────────────────────────────

    /// Intern an arbitrary string (for error messages, etc.).
    pub fn intern_string(&mut self, s: &str) -> StringId {
        if let Some(&id) = self.string_to_id.get(s) {
            return id;
        }
        let id = StringId::from_usize(self.strings.len());
        self.strings.push(s.to_string());
        self.string_to_id.insert(s.to_string(), id);
        id
    }

    /// Get a string by its StringId.
    pub fn string_str(&self, id: StringId) -> &str {
        &self.strings[id.to_usize()]
    }

    // ── Solvable management ──────────────────────────────────────────

    /// Add a package candidate to the pool.
    ///
    /// Returns the assigned `SolvableId`.
    pub fn add_candidate(&mut self, candidate: PackageCandidate) -> SolvableId {
        let name_id = self.intern_name(&candidate.name);
        let solvable_id = SolvableId::from_usize(self.solvables.len());
        self.solvables.push(candidate);
        self.name_to_solvables
            .entry(name_id)
            .or_default()
            .push(solvable_id);
        solvable_id
    }

    /// Get a candidate by SolvableId.
    pub fn candidate(&self, id: SolvableId) -> &PackageCandidate {
        &self.solvables[id.to_usize()]
    }

    /// Get candidates by NameId.
    pub fn candidates_for_name(&self, name_id: NameId) -> Option<&[SolvableId]> {
        self.name_to_solvables.get(&name_id).map(|v| v.as_slice())
    }

    /// Register a candidate under the names of the virtual packages it
    /// provides, so dependencies on those names can be satisfied.
    ///
    /// Version constraints on versioned provides are matched against the
    /// provider's own version (documented simplification until provides
    /// carry their own version sets).
    pub fn add_provides(&mut self, solvable: SolvableId, provides: &[DepConstraint]) {
        for provide in provides {
            let name_id = self.intern_name(&provide.name);
            let entry = self.name_to_solvables.entry(name_id).or_default();
            if !entry.contains(&solvable) {
                entry.push(solvable);
            }
        }
    }

    // ── Version set management ───────────────────────────────────────

    /// Intern a version set (name + constraint pair).
    pub fn intern_version_set(
        &mut self,
        name_id: NameId,
        constraint: DepConstraint,
    ) -> VersionSetId {
        let id = VersionSetId::from_usize(self.version_sets.len());
        self.version_sets.push(VersionSetEntry {
            name_id,
            constraint,
            negated: false,
        });
        id
    }

    /// Intern a conflict version set.
    ///
    /// In resolvo, `constrains` forbids candidates that do **not** match the
    /// version set. For ALPM conflicts we want to forbid candidates that
    /// **do** match, so we store the constraint with `negated = true`.
    pub fn intern_conflict_version_set(
        &mut self,
        name_id: NameId,
        constraint: DepConstraint,
    ) -> VersionSetId {
        let id = VersionSetId::from_usize(self.version_sets.len());
        self.version_sets.push(VersionSetEntry {
            name_id,
            constraint,
            negated: true,
        });
        id
    }

    /// Get a version set entry.
    pub(crate) fn version_set(&self, id: VersionSetId) -> &VersionSetEntry {
        &self.version_sets[id.to_usize()]
    }

    // ── Version set union management ─────────────────────────────────

    /// Intern a version set union.
    pub fn intern_version_set_union(&mut self, members: Vec<VersionSetId>) -> VersionSetUnionId {
        let id = VersionSetUnionId::from_usize(self.version_set_unions.len());
        self.version_set_unions
            .push(VersionSetUnionEntry { members });
        id
    }

    /// Get a version set union entry.
    pub(crate) fn version_set_union(&self, id: VersionSetUnionId) -> &VersionSetUnionEntry {
        &self.version_set_unions[id.to_usize()]
    }
}

impl Default for PackagePool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_name_roundtrip() {
        let mut pool = PackagePool::new();
        let id = pool.intern_name("bash");
        assert_eq!(pool.name_str(id), "bash");
        // Re-interning returns same ID
        assert_eq!(pool.intern_name("bash"), id);
    }

    #[test]
    fn intern_string_roundtrip() {
        let mut pool = PackagePool::new();
        let id = pool.intern_string("some error");
        assert_eq!(pool.string_str(id), "some error");
        assert_eq!(pool.intern_string("some error"), id);
    }

    #[test]
    fn add_candidate_and_lookup() {
        let mut pool = PackagePool::new();
        let sid = pool.add_candidate(PackageCandidate {
            name: "bash".to_string(),
            version: Version::parse("5.2-1"),
            depends: vec![],
            conflicts: vec![],
            provides: vec![],
            optdepends: vec![],
        });

        let c = pool.candidate(sid);
        assert_eq!(c.name, "bash");
        assert_eq!(c.version.to_string(), "5.2-1");
    }

    #[test]
    fn candidates_for_name() {
        let mut pool = PackagePool::new();
        let _s1 = pool.add_candidate(PackageCandidate {
            name: "bash".to_string(),
            version: Version::parse("5.1-1"),
            depends: vec![],
            conflicts: vec![],
            provides: vec![],
            optdepends: vec![],
        });
        let _s2 = pool.add_candidate(PackageCandidate {
            name: "bash".to_string(),
            version: Version::parse("5.2-1"),
            depends: vec![],
            conflicts: vec![],
            provides: vec![],
            optdepends: vec![],
        });

        let name_id = pool.intern_name("bash");
        let candidates = pool.candidates_for_name(name_id).unwrap();
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn version_set_roundtrip() {
        let mut pool = PackagePool::new();
        let name_id = pool.intern_name("glibc");
        let constraint = DepConstraint::parse("glibc>=2.38");
        let vs_id = pool.intern_version_set(name_id, constraint.clone());
        let entry = pool.version_set(vs_id);
        assert_eq!(entry.name_id, name_id);
        assert_eq!(entry.constraint, constraint);
    }
}
