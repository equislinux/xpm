//! Resolvo `DependencyProvider` implementation for xpm.
//!
//! Bridges the xpm package pool to resolvo's solver by implementing the
//! [`Interner`] and [`DependencyProvider`] traits.

use std::any::Any;
use std::fmt;

use resolvo::{
    ArenaId, Candidates, Condition, ConditionId, Dependencies, DependencyProvider,
    HintDependenciesAvailable, Interner, KnownDependencies, NameId, SolvableId, SolverCache,
    StringId, VersionSetId, VersionSetUnionId,
};

use crate::resolver::dependency::DepConstraint;
use crate::resolver::types::{PackageCandidate, PackagePool, VersionSetEntry};

/// xpm's dependency provider for the resolvo SAT solver.
///
/// This struct wraps a [`PackagePool`] and implements both [`Interner`] and
/// [`DependencyProvider`] to feed package data into the solver.
pub struct XpmProvider {
    /// The package pool containing all interned data.
    pub pool: PackagePool,
}

impl XpmProvider {
    /// Create a new provider from an existing package pool.
    pub fn new(pool: PackagePool) -> Self {
        Self { pool }
    }
}

// ── Interner implementation ──────────────────────────────────────────────────

impl Interner for XpmProvider {
    fn display_solvable(&self, solvable: SolvableId) -> impl fmt::Display + '_ {
        let c = self.pool.candidate(solvable);
        DisplaySolvable {
            name: c.name.clone(),
            version: c.version.to_string(),
        }
    }

    fn display_name(&self, name: NameId) -> impl fmt::Display + '_ {
        self.pool.name_str(name)
    }

    fn display_version_set(&self, version_set: VersionSetId) -> impl fmt::Display + '_ {
        let entry = self.pool.version_set(version_set);
        DisplayVersionSet {
            constraint: entry.constraint.clone(),
        }
    }

    fn display_string(&self, string_id: StringId) -> impl fmt::Display + '_ {
        self.pool.string_str(string_id)
    }

    fn version_set_name(&self, version_set: VersionSetId) -> NameId {
        self.pool.version_set(version_set).name_id
    }

    fn solvable_name(&self, solvable: SolvableId) -> NameId {
        let name = &self.pool.candidate(solvable).name;
        // This should always be interned already
        self.pool
            .name_to_id
            .get(name)
            .copied()
            .expect("solvable name not interned")
    }

    fn version_sets_in_union(
        &self,
        version_set_union: VersionSetUnionId,
    ) -> impl Iterator<Item = VersionSetId> {
        self.pool
            .version_set_union(version_set_union)
            .members
            .iter()
            .copied()
    }

    fn resolve_condition(&self, _condition: ConditionId) -> Condition {
        // xpm does not use conditional requirements (yet)
        unreachable!("xpm does not use conditional requirements")
    }
}

// ── DependencyProvider implementation ────────────────────────────────────────

impl DependencyProvider for XpmProvider {
    async fn filter_candidates(
        &self,
        candidates: &[SolvableId],
        version_set: VersionSetId,
        inverse: bool,
    ) -> Vec<SolvableId> {
        let entry = self.pool.version_set(version_set);
        candidates
            .iter()
            .copied()
            .filter(|&sid| {
                let candidate = self.pool.candidate(sid);
                let raw_match = self.candidate_satisfies(candidate, entry);
                // For conflict version sets the match is negated so that
                // resolvo's "non-matching → forbidden" logic correctly
                // forbids the right candidates.
                let matches = if entry.negated { !raw_match } else { raw_match };
                if inverse {
                    !matches
                } else {
                    matches
                }
            })
            .collect()
    }

    async fn get_candidates(&self, name: NameId) -> Option<Candidates> {
        let solvable_ids = self.pool.candidates_for_name(name)?;
        if solvable_ids.is_empty() {
            return None;
        }

        Some(Candidates {
            candidates: solvable_ids.to_vec(),
            favored: self.pool.favored_for(name),
            locked: None,
            hint_dependencies_available: HintDependenciesAvailable::All,
            excluded: Vec::new(),
        })
    }

    async fn sort_candidates(&self, _solver: &SolverCache<Self>, solvables: &mut [SolvableId]) {
        // Highest version first. On equal versions, prefer repository
        // candidates over installed ones so an explicit request can reinstall
        // from the repository; dependencies still use `favored`.
        solvables.sort_by(|&a, &b| {
            let va = &self.pool.candidate(a).version;
            let vb = &self.pool.candidate(b).version;
            vb.cmp(va)
                .then_with(|| self.pool.is_installed(a).cmp(&self.pool.is_installed(b)))
        });
    }

    async fn get_dependencies(&self, solvable: SolvableId) -> Dependencies {
        let candidate = self.pool.candidate(solvable);

        let mut known = KnownDependencies::default();

        // Add runtime dependencies as requirements
        for dep in &candidate.depends {
            // Find or intern the dependency's name and version set
            if let Some(&dep_name_id) = self.pool.name_to_id.get(&dep.name) {
                // Look through existing version sets
                if let Some(vs_id) = self.find_version_set(dep_name_id, dep) {
                    known.requirements.push(vs_id.into());
                }
            }
            // If the dependency name is unknown, we skip it (it will be caught
            // as unsolvable by the solver when no candidates are found).
        }

        // Add conflicts as constrains (using negated version sets).
        //
        // A package never conflicts with itself: canonical providers such as
        // systemd declare `provides = udev` and `conflicts = udev` at the
        // same time. The constraint is name-based, so adding it would forbid
        // the very provider that owns the name. Self-satisfied conflicts are
        // therefore skipped (ALPM implicitly excludes the package itself).
        for conflict in &candidate.conflicts {
            if let Some(&conflict_name_id) = self.pool.name_to_id.get(&conflict.name) {
                if let Some(vs_id) = self.find_conflict_version_set(conflict_name_id, conflict) {
                    if self.candidate_satisfies(candidate, self.pool.version_set(vs_id)) {
                        continue;
                    }
                    known.constrains.push(vs_id);
                }
            }
        }

        Dependencies::Known(known)
    }

    fn should_cancel_with_value(&self) -> Option<Box<dyn Any>> {
        None
    }
}

impl XpmProvider {
    /// Whether `candidate` satisfies the constraint behind `entry`.
    ///
    /// Mirrors libalpm's `_alpm_depcmp`: the package name is checked first
    /// (with the package version), then the `provides` list. An unversioned
    /// dependency accepts any provision with a matching name; a versioned
    /// dependency requires a versioned provision whose version satisfies it.
    fn candidate_satisfies(&self, candidate: &PackageCandidate, entry: &VersionSetEntry) -> bool {
        let target = self.pool.name_str(entry.name_id);

        if candidate.name == target && entry.constraint.matches(&candidate.version) {
            return true;
        }

        candidate.provides.iter().any(|provide| {
            if provide.name != target {
                return false;
            }
            match &entry.constraint.version {
                // Unversioned dependency: any provision with the name works.
                None => true,
                // Versioned dependency: only versioned provisions match.
                Some(_) => provide
                    .version
                    .as_ref()
                    .is_some_and(|version| entry.constraint.matches(version)),
            }
        })
    }

    /// Find a normal (non-negated) version set matching the given constraint.
    fn find_version_set(
        &self,
        name_id: NameId,
        constraint: &DepConstraint,
    ) -> Option<VersionSetId> {
        for (i, entry) in self.pool.version_sets.iter().enumerate() {
            if entry.name_id == name_id && !entry.negated && entry.constraint == *constraint {
                return Some(VersionSetId::from_usize(i));
            }
        }
        None
    }

    /// Find a negated (conflict) version set matching the given constraint.
    fn find_conflict_version_set(
        &self,
        name_id: NameId,
        constraint: &DepConstraint,
    ) -> Option<VersionSetId> {
        for (i, entry) in self.pool.version_sets.iter().enumerate() {
            if entry.name_id == name_id && entry.negated && entry.constraint == *constraint {
                return Some(VersionSetId::from_usize(i));
            }
        }
        None
    }
}

// ── Display helpers ──────────────────────────────────────────────────────────

struct DisplaySolvable {
    name: String,
    version: String,
}

impl fmt::Display for DisplaySolvable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.name, self.version)
    }
}

#[derive(Clone)]
struct DisplayVersionSet {
    constraint: DepConstraint,
}

impl fmt::Display for DisplayVersionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.constraint.op, &self.constraint.version) {
            (Some(op), Some(ver)) => write!(f, "{op}{ver}"),
            _ => write!(f, "*"),
        }
    }
}
