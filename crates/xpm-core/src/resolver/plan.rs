//! High-level resolution: requirements in, dependency-ordered plan out.
//!
//! This is the entry point the CLI (`xpm install`) uses to turn a list of
//! requested package names into a concrete install plan. It builds the
//! [`PackagePool`], feeds the [`XpmProvider`] SAT solver, and returns the
//! solved candidates in dependency order (dependencies before dependents).

use std::collections::{HashMap, HashSet};

use resolvo::{ArenaId, NameId, Problem, Solver, VersionSetId};

use crate::error::{XpmError, XpmResult};
use crate::resolver::dependency::{DepConstraint, Operator};
use crate::resolver::provider::XpmProvider;
use crate::resolver::types::{PackageCandidate, PackagePool};
use crate::resolver::version::Version;

/// A requested package: name plus an optional exact version (`name=version`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    /// Requested package name (may be a virtual name provided by a package).
    pub name: String,
    /// Exact version when the request used `name=version`.
    pub version: Option<Version>,
}

impl Requirement {
    /// Parse a CLI requirement: `foo` means any version, `foo=1.2-1` pins one.
    pub fn parse(spec: &str) -> XpmResult<Self> {
        match spec.split_once('=') {
            Some((name, version)) if !name.is_empty() && !version.is_empty() => Ok(Self {
                name: name.to_string(),
                version: Some(Version::parse(version)),
            }),
            Some(_) => Err(XpmError::Package(format!(
                "invalid package requirement '{spec}' (expected name or name=version)"
            ))),
            None if !spec.is_empty() => Ok(Self {
                name: spec.to_string(),
                version: None,
            }),
            None => Err(XpmError::Package("empty package name".to_string())),
        }
    }

    fn constraint(&self) -> DepConstraint {
        match &self.version {
            Some(version) => DepConstraint {
                name: self.name.clone(),
                op: Some(Operator::Eq),
                version: Some(version.clone()),
            },
            None => DepConstraint {
                name: self.name.clone(),
                op: None,
                version: None,
            },
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
    if requirements.is_empty() {
        return Ok(Vec::new());
    }

    let mut pool = PackagePool::new();
    for candidate in &candidates {
        let solvable = pool.add_candidate(candidate.clone());
        let provides = candidate.provides.clone();
        pool.add_provides(solvable, &provides);
    }

    // Intern dependency and conflict version sets for every candidate.
    for candidate in &candidates {
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
        let satisfiable = candidates.iter().any(|candidate| {
            candidate.name == constraint.name
                || candidate
                    .provides
                    .iter()
                    .any(|provide| provide.name == constraint.name)
        });
        if !satisfiable {
            return Err(XpmError::PackageNotFound {
                name: constraint.name.clone(),
            });
        }
        let name_id = pool.intern_name(&constraint.name);
        let version_set = match find_version_set(&pool, name_id, &constraint, false) {
            Some(id) => id,
            None => pool.intern_version_set(name_id, constraint),
        };
        requirement_ids.push(version_set.into());
    }

    let snapshot = pool.solvables.clone();
    let problem = Problem::new().requirements(requirement_ids);
    let mut solver = Solver::new(XpmProvider::new(pool));
    let solution = solver
        .solve(problem)
        .map_err(|e| XpmError::DependencyConflict(format!("{e:?}")))?;

    let solved: Vec<usize> = solution.iter().map(|sid| sid.to_usize()).collect();
    let solved_set: HashSet<usize> = solved.iter().copied().collect();

    // Index the solution by package name and by the virtual names provided.
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for &idx in &solved {
        by_name.insert(snapshot[idx].name.as_str(), idx);
        for provide in &snapshot[idx].provides {
            by_name.entry(provide.name.as_str()).or_insert(idx);
        }
    }

    let explicit_names: HashSet<&str> = requirements.iter().map(|req| req.name.as_str()).collect();

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
            (candidate, explicit)
        })
        .collect())
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
            depends: deps.iter().map(|d| DepConstraint::parse(d)).collect(),
            conflicts: vec![],
            provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
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
}
