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

/// Resolve `requirements` against `candidates`, returning the install plan in
/// dependency order. The boolean marks packages requested explicitly (`true`)
/// versus pulled in as dependencies (`false`).
///
/// Dependencies may be satisfied by real package names or by unversioned
/// `provides` entries.
pub fn resolve_closure(
    candidates: Vec<PackageCandidate>,
    requirements: &[Requirement],
) -> XpmResult<Vec<(PackageCandidate, bool)>> {
    Ok(
        resolve_closure_with_installed(candidates, Vec::new(), requirements)?
            .into_iter()
            .map(|(candidate, explicit, _installed)| (candidate, explicit))
            .collect(),
    )
}

/// Resolve `requirements` while treating `installed` as already-present
/// packages.
///
/// Installed candidates are added to the pool and marked as favored for their
/// package name and every name they provide, except for names explicitly
/// requested by the user. This keeps satisfied dependencies in place (like
/// pacman) while still allowing the solver to pick a newer repository
/// candidate when a version constraint requires it.
///
/// The returned boolean pair is `(explicit, installed)`. Callers should skip
/// installed entries that are not being changed.
pub fn resolve_closure_with_installed(
    candidates: Vec<PackageCandidate>,
    installed: Vec<PackageCandidate>,
    requirements: &[Requirement],
) -> XpmResult<Vec<(PackageCandidate, bool, bool)>> {
    if requirements.is_empty() {
        return Ok(Vec::new());
    }

    let explicit_names: HashSet<&str> = requirements.iter().map(|req| req.name.as_str()).collect();

    // Repository-qualified requests (`repo/name`) restrict both the literal
    // package and its repository. Providers are not considered for these
    // requests: the user explicitly asked for a package in that repository.
    // When the same name is also requested unqualified, the restriction is
    // dropped for that name.
    let mut repo_scoped: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut unscoped_names: HashSet<&str> = HashSet::new();
    for requirement in requirements {
        match requirement.repo.as_deref() {
            Some(repo) => {
                repo_scoped
                    .entry(requirement.name.as_str())
                    .or_default()
                    .insert(repo);
            }
            None => {
                unscoped_names.insert(requirement.name.as_str());
            }
        }
    }
    let repo_only: HashMap<&str, &HashSet<&str>> = repo_scoped
        .iter()
        .filter(|(name, _)| !unscoped_names.contains(*name))
        .map(|(name, repos)| (*name, repos))
        .collect();

    for requirement in requirements.iter().filter(|req| req.repo.is_some()) {
        let repo = requirement.repo.as_deref().unwrap_or_default();
        let constraint = requirement.constraint();
        let found = candidates.iter().any(|candidate| {
            candidate.name == requirement.name
                && candidate.repo.as_deref() == Some(repo)
                && constraint.matches(&candidate.version)
        });
        if !found {
            return Err(XpmError::PackageNotFound {
                name: requirement.display(),
            });
        }
    }

    // Exclude candidates that conflict with an installed package. The solver
    // does not model packages that remain installed but are not part of the
    // solution, so this check has to happen before it runs. Explicitly
    // requested names are skipped: the caller asked to replace/upgrade them,
    // and the final dependency check will still report real conflicts.
    let mut blocked: Vec<(PackageCandidate, String, DepConstraint)> = Vec::new();
    let mut usable_candidates = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        match conflict_with_installed(&candidate, &installed, &explicit_names) {
            Some((installed_name, conflict)) => {
                blocked.push((candidate, installed_name, conflict));
            }
            None => usable_candidates.push(candidate),
        }
    }
    let candidates = usable_candidates;

    let mut pool = PackagePool::new();
    for candidate in &candidates {
        if let Some(repos) = repo_only.get(candidate.name.as_str()) {
            if !candidate
                .repo
                .as_deref()
                .is_some_and(|repo| repos.contains(repo))
            {
                continue;
            }
        }
        let solvable = pool.add_candidate(candidate.clone());
        let provides = filtered_provides(candidate, &repo_only);
        pool.add_provides(solvable, &provides);
    }

    for candidate in &installed {
        // An installed package cannot satisfy `repo/name`; force the
        // repository candidate so the request keeps its meaning.
        if repo_only.contains_key(candidate.name.as_str()) {
            continue;
        }
        let solvable = pool.add_candidate(candidate.clone());
        let provides = filtered_provides(candidate, &repo_only);
        pool.add_provides(solvable, &provides);
        pool.mark_installed(solvable);

        if !explicit_names.contains(candidate.name.as_str()) {
            if let Some(name_id) = pool.name_to_id.get(&candidate.name).copied() {
                pool.set_favored(name_id, solvable);
            }
        }
        for provide in &provides {
            if !explicit_names.contains(provide.name.as_str()) {
                if let Some(name_id) = pool.name_to_id.get(&provide.name).copied() {
                    pool.set_favored(name_id, solvable);
                }
            }
        }
    }

    // Intern dependency and conflict version sets for every candidate.
    for candidate in candidates.iter().chain(installed.iter()) {
        for dep in &candidate.depends {
            intern_once(&mut pool, dep, false);
        }
        for conflict in &candidate.conflicts {
            intern_once(&mut pool, conflict, true);
        }
    }

    // Reject requirements that nothing can satisfy before invoking the solver.
    let mut requirement_ids = Vec::new();
    for requirement in requirements {
        let constraint = requirement.constraint();
        let satisfiable = candidates
            .iter()
            .chain(installed.iter())
            .any(|candidate| requirement_satisfied_by(candidate, requirement));
        if !satisfiable {
            if let Some((candidate, installed_name, conflict)) = blocked
                .iter()
                .find(|(candidate, _, _)| requirement_satisfied_by(candidate, requirement))
            {
                return Err(XpmError::DependencyConflict(format!(
                    "cannot install {}: {} conflicts with installed package {} ('{}')",
                    requirement.display(),
                    candidate.name,
                    installed_name,
                    conflict
                )));
            }
            return Err(XpmError::PackageNotFound {
                name: requirement.display(),
            });
        }
        let name_id = pool.intern_name(&constraint.name);
        let version_set = match find_version_set(&pool, name_id, &constraint, false) {
            Some(id) => id,
            None => pool.intern_version_set(name_id, constraint),
        };
        requirement_ids.push(version_set.into());
    }

    let installed_indexes: HashSet<usize> = (0..pool.solvables.len())
        .filter(|&index| pool.is_installed(SolvableId::from_usize(index)))
        .collect();
    let snapshot = pool.solvables.clone();
    let problem = Problem::new().requirements(requirement_ids);
    let mut solver = Solver::new(XpmProvider::new(pool));
    let solution = solver
        .solve(problem)
        .map_err(|e| XpmError::DependencyConflict(format!("{e:?}")))?;

    let solved: Vec<usize> = solution.iter().map(|sid| sid.to_usize()).collect();
    let solved_set: HashSet<usize> = solved.iter().copied().collect();

    check_solution_conflicts(&solved, &solved_set, &snapshot, &installed_indexes)?;

    // Index the solution by package name and by the virtual names provided.
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for &idx in &solved {
        by_name.insert(snapshot[idx].name.as_str(), idx);
        for provide in &snapshot[idx].provides {
            by_name.entry(provide.name.as_str()).or_insert(idx);
        }
    }

    let mut visited = HashSet::new();
    let mut order = Vec::new();
    for &idx in &solved {
        order_dfs(
            idx,
            &snapshot,
            &by_name,
            &solved_set,
            &mut visited,
            &mut order,
        );
    }

    Ok(order
        .into_iter()
        .map(|idx| {
            let candidate = snapshot[idx].clone();
            let explicit = explicit_names.contains(candidate.name.as_str())
                || candidate
                    .provides
                    .iter()
                    .any(|provide| explicit_names.contains(provide.name.as_str()));
            let is_installed = installed_indexes.contains(&idx);
            (candidate, explicit, is_installed)
        })
        .collect())
}

/// Provides to register for a candidate, excluding names requested as
/// `repo/name` (those must resolve to the literal package in that repository).
fn filtered_provides(
    candidate: &PackageCandidate,
    repo_only: &HashMap<&str, &HashSet<&str>>,
) -> Vec<DepConstraint> {
    candidate
        .provides
        .iter()
        .filter(|provide| !repo_only.contains_key(provide.name.as_str()))
        .cloned()
        .collect()
}

/// Whether `candidate` satisfies a CLI requirement (name/provides, version and
/// repository qualifier).
fn requirement_satisfied_by(candidate: &PackageCandidate, requirement: &Requirement) -> bool {
    let constraint = requirement.constraint();
    match requirement.repo.as_deref() {
        Some(repo) => {
            candidate.repo.as_deref() == Some(repo)
                && candidate.name == requirement.name
                && constraint.matches(&candidate.version)
        }
        None => package_satisfies(candidate, &constraint),
    }
}

/// ALPM `depcmp`: package name first, then `provides`. A versioned dependency
/// only accepts a versioned provision whose version satisfies the constraint.
fn package_satisfies(candidate: &PackageCandidate, dependency: &DepConstraint) -> bool {
    if candidate.name == dependency.name && dependency.matches(&candidate.version) {
        return true;
    }

    candidate.provides.iter().any(|provide| {
        if provide.name != dependency.name {
            return false;
        }
        match &dependency.version {
            None => true,
            Some(_) => provide
                .version
                .as_ref()
                .is_some_and(|version| dependency.matches(version)),
        }
    })
}

/// Return the first installed package that `candidate` conflicts with, unless
/// the installed package is explicitly requested in the same transaction or
/// is replaced by the candidate.
fn conflict_with_installed(
    candidate: &PackageCandidate,
    installed: &[PackageCandidate],
    explicit_names: &HashSet<&str>,
) -> Option<(String, DepConstraint)> {
    for package in installed {
        if package.name == candidate.name
            || explicit_names.contains(package.name.as_str())
            || candidate
                .replaces
                .iter()
                .any(|replaces| package_satisfies(package, replaces))
        {
            continue;
        }
        if let Some(conflict) = candidate
            .conflicts
            .iter()
            .find(|conflict| package_satisfies(package, conflict))
        {
            return Some((package.name.clone(), conflict.clone()));
        }
    }
    None
}

/// Reject solutions where two packages that will be installed conflict with
/// each other, or where a new package conflicts with a package that stays
/// installed. Self-conflicting `provides`/`conflicts` pairs (e.g. systemd and
/// udev) are skipped because ALPM excludes a package from its own conflict.
fn check_solution_conflicts(
    solved: &[usize],
    solved_set: &HashSet<usize>,
    snapshot: &[PackageCandidate],
    installed_indexes: &HashSet<usize>,
) -> XpmResult<()> {
    for &index in solved {
        if installed_indexes.contains(&index) {
            continue;
        }
        let candidate = &snapshot[index];

        for &other_index in solved {
            if other_index == index {
                continue;
            }
            let other = &snapshot[other_index];
            if other.name == candidate.name {
                continue;
            }
            if let Some(conflict) = candidate
                .conflicts
                .iter()
                .find(|conflict| package_satisfies(other, conflict))
            {
                return Err(XpmError::DependencyConflict(format!(
                    "installing {} conflicts with {} ('{}')",
                    candidate.name, other.name, conflict
                )));
            }
        }

        for (other_index, other) in snapshot.iter().enumerate() {
            if !installed_indexes.contains(&other_index)
                || solved_set.contains(&other_index)
                || other.name == candidate.name
            {
                continue;
            }
            if let Some(conflict) = candidate
                .conflicts
                .iter()
                .find(|conflict| package_satisfies(other, conflict))
            {
                return Err(XpmError::DependencyConflict(format!(
                    "installing {} conflicts with installed package {} ('{}')",
                    candidate.name, other.name, conflict
                )));
            }
        }
    }

    Ok(())
}

fn intern_once(pool: &mut PackagePool, constraint: &DepConstraint, negated: bool) {
    let name_id = pool.intern_name(&constraint.name);
    if find_version_set(pool, name_id, constraint, negated).is_none() {
        if negated {
            pool.intern_conflict_version_set(name_id, constraint.clone());
        } else {
            pool.intern_version_set(name_id, constraint.clone());
        }
    }
}

fn find_version_set(
    pool: &PackagePool,
    name_id: NameId,
    constraint: &DepConstraint,
    negated: bool,
) -> Option<VersionSetId> {
    pool.version_sets
        .iter()
        .enumerate()
        .find(|(_, entry)| {
            entry.name_id == name_id && entry.negated == negated && entry.constraint == *constraint
        })
        .map(|(index, _)| VersionSetId::from_usize(index))
}

#[allow(clippy::too_many_arguments)]
fn order_dfs(
    idx: usize,
    snapshot: &[PackageCandidate],
    by_name: &HashMap<&str, usize>,
    solved_set: &HashSet<usize>,
    visited: &mut HashSet<usize>,
    out: &mut Vec<usize>,
) {
    if !visited.insert(idx) {
        return;
    }
    for dep in &snapshot[idx].depends {
        if let Some(&dep_idx) = by_name.get(dep.name.as_str()) {
            if solved_set.contains(&dep_idx) {
                order_dfs(dep_idx, snapshot, by_name, solved_set, visited, out);
            }
        }
    }
    out.push(idx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, version: &str, deps: &[&str], provides: &[&str]) -> PackageCandidate {
        PackageCandidate {
            name: name.to_string(),
            version: Version::parse(version),
            repo: None,
            depends: deps.iter().map(|d| DepConstraint::parse(d)).collect(),
            conflicts: vec![],
            provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
            replaces: vec![],
            optdepends: vec![],
        }
    }

    fn plan_names(plan: &[(PackageCandidate, bool)]) -> Vec<String> {
        plan.iter()
            .map(|(candidate, _)| format!("{}={}", candidate.name, candidate.version))
            .collect()
    }

    #[test]
    fn resolves_transitive_dependencies_in_order() {
        let candidates = vec![
            candidate("app", "1.0-1", &["lib", "runtime"], &[]),
            candidate("lib", "2.0-1", &["runtime"], &[]),
            candidate("runtime", "3.0-1", &[], &[]),
        ];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        let names = plan_names(&plan);

        let runtime = names
            .iter()
            .position(|n| n.starts_with("runtime="))
            .unwrap();
        let lib = names.iter().position(|n| n.starts_with("lib=")).unwrap();
        let app = names.iter().position(|n| n.starts_with("app=")).unwrap();
        assert!(runtime < lib, "runtime must precede lib: {names:?}");
        assert!(lib < app, "lib must precede app: {names:?}");
    }

    #[test]
    fn marks_requested_packages_explicit_and_dependencies_not() {
        let candidates = vec![
            candidate("app", "1.0-1", &["lib"], &[]),
            candidate("lib", "1.0-1", &[], &[]),
        ];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        let app = plan.iter().find(|(c, _)| c.name == "app").unwrap();
        let lib = plan.iter().find(|(c, _)| c.name == "lib").unwrap();
        assert!(app.1, "requested package is explicit");
        assert!(!lib.1, "pulled dependency is not explicit");
    }

    #[test]
    fn exact_version_requirement_selects_that_candidate() {
        let candidates = vec![
            candidate("tool", "1.0-1", &[], &[]),
            candidate("tool", "2.0-1", &[], &[]),
        ];
        let requests = vec![Requirement::parse("tool=1.0-1").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.version, Version::parse("1.0-1"));
    }

    #[test]
    fn unversioned_provides_satisfy_dependencies() {
        let candidates = vec![
            candidate("app", "1.0-1", &["sh"], &[]),
            candidate("bash", "5.2-1", &[], &["sh"]),
        ];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        let names = plan_names(&plan);
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names[0].starts_with("bash="), "{names:?}");
    }

    #[test]
    fn requesting_a_virtual_name_marks_the_provider_explicit() {
        let candidates = vec![candidate("openssh", "9.0-1", &[], &["ssh"])];
        let requests = vec![Requirement::parse("ssh").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.name, "openssh");
        assert!(plan[0].1);
    }

    #[test]
    fn unknown_package_is_reported() {
        let candidates = vec![candidate("app", "1.0-1", &[], &[])];
        let requests = vec![Requirement::parse("missing").unwrap()];

        let error = resolve_closure(candidates, &requests).unwrap_err();
        assert!(matches!(error, XpmError::PackageNotFound { name } if name == "missing"));
    }

    #[test]
    fn unsatisfiable_dependency_is_reported() {
        let candidates = vec![candidate("app", "1.0-1", &["ghost"], &[])];
        let requests = vec![Requirement::parse("app").unwrap()];

        let error = resolve_closure(candidates, &requests).unwrap_err();
        assert!(matches!(error, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn versioned_provides_satisfy_versioned_dependencies() {
        // Real Arch pattern: bash depends on the readline soname, and the
        // readline package provides it with its own version, unrelated to
        // the package version.
        let candidates = vec![
            candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
            candidate("libfoo", "1.2.3-1", &[], &["libfoo.so=8-64"]),
        ];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        let names = plan_names(&plan);
        assert_eq!(names, vec!["libfoo=1.2.3-1", "app=1.0-1"]);
    }

    #[test]
    fn versioned_provides_reject_mismatched_versions() {
        let candidates = vec![
            candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
            candidate("libfoo", "1.2.3-1", &[], &["libfoo.so=7-64"]),
        ];
        let requests = vec![Requirement::parse("app").unwrap()];

        let error = resolve_closure(candidates, &requests).unwrap_err();
        assert!(matches!(error, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn unversioned_provide_does_not_satisfy_versioned_dependency() {
        let candidates = vec![
            candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
            candidate("libfoo", "1.2.3-1", &[], &["libfoo.so"]),
        ];
        let requests = vec![Requirement::parse("app").unwrap()];

        let error = resolve_closure(candidates, &requests).unwrap_err();
        assert!(matches!(error, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn unversioned_dependency_accepts_versioned_provide() {
        let candidates = vec![
            candidate("app", "1.0-1", &["sh"], &[]),
            candidate("bash", "5.3-1", &[], &["sh=5.3"]),
        ];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        let names = plan_names(&plan);
        assert_eq!(names, vec!["bash=5.3-1", "app=1.0-1"]);
    }

    #[test]
    fn conflicts_match_versioned_provides() {
        let mut conflicting = candidate("other", "1.0-1", &["libfoo.so=8-64"], &[]);
        conflicting.conflicts = vec![DepConstraint::parse("libfoo.so=8-64")];

        let candidates = vec![
            candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
            candidate("libfoo", "1.2.3-1", &[], &["libfoo.so=8-64"]),
            conflicting,
        ];
        let requests = vec![
            Requirement::parse("app").unwrap(),
            Requirement::parse("other").unwrap(),
        ];

        let error = resolve_closure(candidates, &requests).unwrap_err();
        assert!(matches!(error, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn installed_dependency_is_kept_even_when_repo_has_newer() {
        let candidates = vec![
            candidate("app", "1.0-1", &["lib"], &[]),
            candidate("lib", "3.0-1", &[], &[]),
        ];
        let installed = vec![candidate("lib", "2.0-1", &[], &[])];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
        let install: Vec<_> = plan
            .iter()
            .filter(|(_, _, installed)| !*installed)
            .map(|(c, _, _)| c.name.clone())
            .collect();
        assert_eq!(install, vec!["app"]);
        assert!(plan
            .iter()
            .any(|(c, _, installed)| c.name == "lib" && *installed));
    }

    #[test]
    fn installed_version_is_replaced_when_constraint_requires_newer() {
        let candidates = vec![
            candidate("app", "1.0-1", &["lib>=3"], &[]),
            candidate("lib", "3.0-1", &[], &[]),
        ];
        let installed = vec![candidate("lib", "2.0-1", &[], &[])];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
        let lib = plan.iter().find(|(c, _, _)| c.name == "lib").unwrap();
        assert_eq!(lib.0.version, Version::parse("3.0-1"));
        assert!(!lib.2, "the newer repository candidate must be selected");
    }

    #[test]
    fn explicit_request_prefers_newer_repository_candidate() {
        let candidates = vec![candidate("foo", "2.0-1", &[], &[])];
        let installed = vec![candidate("foo", "1.0-1", &[], &[])];
        let requests = vec![Requirement::parse("foo").unwrap()];

        let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.version, Version::parse("2.0-1"));
        assert!(!plan[0].2);
    }

    #[test]
    fn explicit_request_does_not_downgrade_below_installed() {
        let candidates = vec![candidate("foo", "2.0-1", &[], &[])];
        let installed = vec![candidate("foo", "3.0-1", &[], &[])];
        let requests = vec![Requirement::parse("foo").unwrap()];

        let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.version, Version::parse("3.0-1"));
        assert!(plan[0].2);
    }

    #[test]
    fn installed_provide_satisfies_dependency() {
        let candidates = vec![
            candidate("app", "1.0-1", &["sh"], &[]),
            candidate("bash", "6.0-1", &[], &["sh"]),
        ];
        let installed = vec![candidate("bash", "5.3-1", &[], &["sh"])];
        let requests = vec![Requirement::parse("app").unwrap()];

        let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
        let bash = plan.iter().find(|(c, _, _)| c.name == "bash").unwrap();
        assert_eq!(bash.0.version, Version::parse("5.3-1"));
        assert!(bash.2);
    }

    #[test]
    fn requirement_parser_rejects_empty_specs() {
        assert!(matches!(Requirement::parse(""), Err(XpmError::Package(_))));
        assert!(matches!(
            Requirement::parse("=1.0"),
            Err(XpmError::Package(_))
        ));
        assert!(matches!(
            Requirement::parse("foo="),
            Err(XpmError::Package(_))
        ));
        assert!(Requirement::parse("foo=1.0").is_ok());
    }

    #[test]
    fn requirement_parser_accepts_repo_qualified_specs() {
        let req = Requirement::parse("extra/power-profiles-daemon").unwrap();
        assert_eq!(req.repo.as_deref(), Some("extra"));
        assert_eq!(req.name, "power-profiles-daemon");
        assert!(req.version.is_none());

        let pinned = Requirement::parse("extra/power-profiles-daemon=0.30-1").unwrap();
        assert_eq!(pinned.repo.as_deref(), Some("extra"));
        assert_eq!(pinned.name, "power-profiles-daemon");
        assert_eq!(pinned.version, Some(Version::parse("0.30-1")));

        assert!(matches!(
            Requirement::parse("extra/"),
            Err(XpmError::Package(_))
        ));
        assert!(matches!(
            Requirement::parse("/foo"),
            Err(XpmError::Package(_))
        ));
    }

    #[test]
    fn exact_name_beats_provider() {
        // power-profiles-daemon (real) vs tuned-ppd (provides it).
        let candidates = vec![
            candidate("power-profiles-daemon", "0.30-1", &[], &[]),
            candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]),
            candidate("tuned", "2.28.0-1", &[], &[]),
        ];
        let requests = vec![Requirement::parse("power-profiles-daemon").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        let names = plan_names(&plan);
        assert_eq!(names, vec!["power-profiles-daemon=0.30-1"]);
    }

    #[test]
    fn pinned_exact_name_beats_versioned_provider() {
        let candidates = vec![
            candidate("power-profiles-daemon", "0.30-1", &[], &[]),
            candidate(
                "tuned-ppd",
                "2.28.0-1",
                &[],
                &["power-profiles-daemon=0.30-1"],
            ),
        ];
        let requests = vec![Requirement::parse("power-profiles-daemon=0.30-1").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.name, "power-profiles-daemon");
    }

    #[test]
    fn exact_name_beats_installed_provider() {
        let candidates = vec![candidate("power-profiles-daemon", "0.30-1", &[], &[])];
        let installed = vec![candidate(
            "tuned-ppd",
            "2.28.0-1",
            &[],
            &["power-profiles-daemon"],
        )];
        let requests = vec![Requirement::parse("power-profiles-daemon").unwrap()];

        let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.name, "power-profiles-daemon");
        assert!(!plan[0].2, "the repository package must be installed");
    }

    #[test]
    fn provider_is_used_when_no_exact_name_exists() {
        let candidates = vec![
            candidate("provider-a", "1.0-1", &[], &["virtual"]),
            candidate("provider-b", "2.0-1", &[], &["virtual"]),
        ];
        let requests = vec![Requirement::parse("virtual").unwrap()];

        let plan = resolve_closure(candidates, &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.name, "provider-b");
        assert!(plan[0].1, "the provider is the explicit target");
    }

    #[test]
    fn repo_qualified_request_selects_the_named_repo() {
        let mut core = candidate("foo", "1.0-1", &[], &[]);
        core.repo = Some("core".to_string());
        let mut extra = candidate("foo", "1.0-1", &[], &[]);
        extra.repo = Some("extra".to_string());

        let requests = vec![Requirement::parse("extra/foo").unwrap()];
        let plan = resolve_closure(vec![core, extra], &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.repo.as_deref(), Some("extra"));
    }

    #[test]
    fn repo_qualified_request_does_not_accept_providers() {
        let mut provider = candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]);
        provider.repo = Some("extra".to_string());

        let requests = vec![Requirement::parse("extra/power-profiles-daemon").unwrap()];
        let error = resolve_closure(vec![provider], &requests).unwrap_err();
        assert!(matches!(error, XpmError::PackageNotFound { .. }));
    }

    #[test]
    fn provider_conflicting_with_installed_is_not_selected() {
        let mut conflicting = candidate("tuned-ppd", "2.28.0-1", &[], &["ppd-api"]);
        conflicting.conflicts = vec![DepConstraint::parse("power-profiles-daemon")];
        let alternative = candidate("other-ppd", "1.0-1", &[], &["ppd-api"]);
        let installed = vec![candidate("power-profiles-daemon", "0.30-1", &[], &[])];

        let requests = vec![Requirement::parse("ppd-api").unwrap()];
        let plan =
            resolve_closure_with_installed(vec![conflicting, alternative], installed, &requests)
                .unwrap();

        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.name, "other-ppd");
        assert!(!plan[0].2);
    }

    #[test]
    fn blocked_requirement_reports_the_installed_conflict() {
        let mut conflicting = candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]);
        conflicting.conflicts = vec![DepConstraint::parse("power-profiles-daemon")];
        let installed = vec![candidate("power-profiles-daemon", "0.30-1", &[], &[])];

        let requests = vec![Requirement::parse("tuned-ppd").unwrap()];
        let error =
            resolve_closure_with_installed(vec![conflicting], installed, &requests).unwrap_err();
        match error {
            XpmError::DependencyConflict(message) => {
                assert!(message.contains("power-profiles-daemon"), "{message}");
                assert!(message.contains("conflicts"), "{message}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn parses_alpm_version_operators_in_requirements() {
        let ge = Requirement::parse("foo>=1.2-1").unwrap();
        assert_eq!(ge.name, "foo");
        assert_eq!(ge.op, Some(Operator::Ge));
        assert_eq!(ge.version.as_ref().unwrap().to_string(), "1.2-1");

        let gt = Requirement::parse("extra/foo>2.0").unwrap();
        assert_eq!(gt.repo.as_deref(), Some("extra"));
        assert_eq!(gt.op, Some(Operator::Gt));
        assert!(Requirement::parse("foo=").is_err());
    }

    #[test]
    fn operator_requirement_matches_a_newer_installed_candidate() {
        let pacman = candidate("pacman", "7.1.0-1", &[], &[]);
        let requests = vec![Requirement::parse("pacman>6.1").unwrap()];
        let plan = resolve_closure(vec![pacman], &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.name, "pacman");
    }

    #[test]
    fn provider_tie_break_prefers_repository_priority_then_name() {
        let mut core = candidate("zzz-core-provider", "1.0-1", &[], &["virtual"]);
        core.repo = Some("core".to_string());
        let mut extra = candidate("aaa-extra-provider", "1.0-1", &[], &["virtual"]);
        extra.repo = Some("extra".to_string());

        // `core` is inserted first, so it has repository priority over `extra`
        // even though the other provider sorts first by name.
        let requests = vec![Requirement::parse("virtual").unwrap()];
        let plan = resolve_closure(vec![core, extra], &requests).unwrap();
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].0.name, "zzz-core-provider");
        assert_eq!(plan[0].0.repo.as_deref(), Some("core"));
    }

    #[test]
    fn self_providing_conflict_between_targets_is_reported() {
        // tuned-ppd provides and conflicts power-profiles-daemon; the resolver
        // skips the self-conflict, so the post-solve check must catch the pair.
        let mut tuned_ppd = candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]);
        tuned_ppd.conflicts = vec![DepConstraint::parse("power-profiles-daemon")];
        let ppd = candidate("power-profiles-daemon", "0.30-1", &[], &[]);

        let requests = vec![
            Requirement::parse("tuned-ppd").unwrap(),
            Requirement::parse("power-profiles-daemon").unwrap(),
        ];
        let error = resolve_closure(vec![tuned_ppd, ppd], &requests).unwrap_err();
        assert!(matches!(error, XpmError::DependencyConflict(_)));
    }
}
