//! Integration tests for the dependency resolver.
//!
//! Tests the full pipeline: package pool → provider → SAT solver → solution.

use resolvo::{ArenaId, Problem, Solver};

use super::dependency::DepConstraint;
use super::provider::XpmProvider;
use super::types::{PackageCandidate, PackagePool};
use super::version::Version;

/// Helper: create a simple candidate with no deps/conflicts.
fn simple(name: &str, ver: &str) -> PackageCandidate {
    PackageCandidate {
        name: name.to_string(),
        version: Version::parse(ver),
        depends: vec![],
        conflicts: vec![],
        provides: vec![],
        optdepends: vec![],
    }
}

/// Helper: create a candidate with dependencies.
fn with_deps(name: &str, ver: &str, deps: &[&str]) -> PackageCandidate {
    PackageCandidate {
        name: name.to_string(),
        version: Version::parse(ver),
        depends: deps.iter().map(|d| DepConstraint::parse(d)).collect(),
        conflicts: vec![],
        provides: vec![],
        optdepends: vec![],
    }
}

/// Helper: create a candidate with dependencies and conflicts.
fn with_deps_and_conflicts(
    name: &str,
    ver: &str,
    deps: &[&str],
    conflicts: &[&str],
) -> PackageCandidate {
    PackageCandidate {
        name: name.to_string(),
        version: Version::parse(ver),
        depends: deps.iter().map(|d| DepConstraint::parse(d)).collect(),
        conflicts: conflicts.iter().map(|c| DepConstraint::parse(c)).collect(),
        provides: vec![],
        optdepends: vec![],
    }
}

/// Build a provider from candidates, interning all necessary version sets
/// for dependencies and conflicts.
fn build_provider(candidates: Vec<PackageCandidate>) -> XpmProvider {
    let mut pool = PackagePool::new();

    // First pass: add all candidates to the pool
    for c in &candidates {
        pool.add_candidate(c.clone());
    }

    // Second pass: intern version sets for all dependencies and conflicts
    for c in &candidates {
        for dep in &c.depends {
            let dep_name_id = pool.intern_name(&dep.name);
            pool.intern_version_set(dep_name_id, dep.clone());
        }
        for conflict in &c.conflicts {
            let conflict_name_id = pool.intern_name(&conflict.name);
            pool.intern_conflict_version_set(conflict_name_id, conflict.clone());
        }
    }

    XpmProvider::new(pool)
}

/// Resolve the given package names and return sorted solution names+versions.
fn solve(provider: XpmProvider, package_names: &[&str]) -> Result<Vec<String>, String> {
    // Build requirements: for each requested package, find the unconstrained
    // version set that was pre-interned.
    let mut requirements = Vec::new();
    for &name in package_names {
        if let Some(&name_id) = provider.pool.name_to_id.get(name) {
            let constraint = DepConstraint::parse(name);
            let mut found = None;
            for (i, entry) in provider.pool.version_sets.iter().enumerate() {
                if entry.name_id == name_id && !entry.negated && entry.constraint == constraint {
                    found = Some(resolvo::VersionSetId::from_usize(i));
                    break;
                }
            }
            if let Some(vs_id) = found {
                requirements.push(vs_id.into());
            }
        }
    }

    let problem = Problem::new().requirements(requirements);

    // Collect candidate data upfront so we can use it after solver consumes provider
    let candidates_snapshot: Vec<(String, String)> = provider
        .pool
        .solvables
        .iter()
        .map(|c| (c.name.clone(), c.version.to_string()))
        .collect();

    let mut solver = Solver::new(provider);
    match solver.solve(problem) {
        Ok(solvable_ids) => {
            let mut result: Vec<String> = solvable_ids
                .iter()
                .map(|&sid| {
                    let idx = sid.to_usize();
                    let (ref name, ref ver) = candidates_snapshot[idx];
                    format!("{}-{}", name, ver)
                })
                .collect();
            result.sort();
            Ok(result)
        }
        Err(_) => Err("dependency resolution failed".to_string()),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn solve_single_package_no_deps() {
    let mut provider = build_provider(vec![simple("bash", "5.2-1")]);
    // Intern unconstrained version set for "bash"
    let name_id = provider.pool.intern_name("bash");
    provider
        .pool
        .intern_version_set(name_id, DepConstraint::parse("bash"));

    let result = solve(provider, &["bash"]).unwrap();
    assert_eq!(result, vec!["bash-5.2-1"]);
}

#[test]
fn solve_picks_highest_version() {
    let mut provider = build_provider(vec![
        simple("vim", "9.0-1"),
        simple("vim", "9.1-1"),
        simple("vim", "8.2-1"),
    ]);
    let name_id = provider.pool.intern_name("vim");
    provider
        .pool
        .intern_version_set(name_id, DepConstraint::parse("vim"));

    let result = solve(provider, &["vim"]).unwrap();
    assert_eq!(result, vec!["vim-9.1-1"]);
}

#[test]
fn solve_with_dependency() {
    let mut provider = build_provider(vec![
        with_deps("firefox", "120.0-1", &["glibc>=2.38"]),
        simple("glibc", "2.38-1"),
        simple("glibc", "2.37-1"),
    ]);

    // Intern unconstrained version sets for the root request
    let firefox_id = provider.pool.intern_name("firefox");
    provider
        .pool
        .intern_version_set(firefox_id, DepConstraint::parse("firefox"));

    let result = solve(provider, &["firefox"]).unwrap();
    assert!(result.contains(&"firefox-120.0-1".to_string()));
    assert!(result.contains(&"glibc-2.38-1".to_string()));
    // glibc 2.37 should NOT be selected (doesn't satisfy >=2.38)
    assert!(!result.contains(&"glibc-2.37-1".to_string()));
}

#[test]
fn solve_dependency_chain() {
    // A depends on B, B depends on C
    let mut provider = build_provider(vec![
        with_deps("app", "1.0-1", &["libfoo"]),
        with_deps("libfoo", "2.0-1", &["libbar"]),
        simple("libbar", "3.0-1"),
    ]);

    let app_id = provider.pool.intern_name("app");
    provider
        .pool
        .intern_version_set(app_id, DepConstraint::parse("app"));

    let result = solve(provider, &["app"]).unwrap();
    assert_eq!(result.len(), 3);
    assert!(result.contains(&"app-1.0-1".to_string()));
    assert!(result.contains(&"libfoo-2.0-1".to_string()));
    assert!(result.contains(&"libbar-3.0-1".to_string()));
}

#[test]
fn solve_multiple_root_packages() {
    let mut provider = build_provider(vec![simple("bash", "5.2-1"), simple("vim", "9.1-1")]);

    let bash_id = provider.pool.intern_name("bash");
    provider
        .pool
        .intern_version_set(bash_id, DepConstraint::parse("bash"));
    let vim_id = provider.pool.intern_name("vim");
    provider
        .pool
        .intern_version_set(vim_id, DepConstraint::parse("vim"));

    let result = solve(provider, &["bash", "vim"]).unwrap();
    assert_eq!(result.len(), 2);
    assert!(result.contains(&"bash-5.2-1".to_string()));
    assert!(result.contains(&"vim-9.1-1".to_string()));
}

#[test]
fn solve_unsatisfiable_dependency() {
    // App requires glibc>=3.0, but only 2.38 is available
    let mut provider = build_provider(vec![
        with_deps("app", "1.0-1", &["glibc>=3.0"]),
        simple("glibc", "2.38-1"),
    ]);

    let app_id = provider.pool.intern_name("app");
    provider
        .pool
        .intern_version_set(app_id, DepConstraint::parse("app"));

    let result = solve(provider, &["app"]);
    assert!(result.is_err());
}

#[test]
fn solve_with_conflict() {
    // pkg-a conflicts with pkg-b, both are requested → should fail
    let mut provider = build_provider(vec![
        with_deps_and_conflicts("pkg-a", "1.0-1", &[], &["pkg-b"]),
        simple("pkg-b", "1.0-1"),
    ]);

    let a_id = provider.pool.intern_name("pkg-a");
    provider
        .pool
        .intern_version_set(a_id, DepConstraint::parse("pkg-a"));
    let b_id = provider.pool.intern_name("pkg-b");
    provider
        .pool
        .intern_version_set(b_id, DepConstraint::parse("pkg-b"));

    let result = solve(provider, &["pkg-a", "pkg-b"]);
    assert!(result.is_err());
}

#[test]
fn solve_version_constrained_dependency() {
    // App requires python>=3.11, multiple python versions available
    let mut provider = build_provider(vec![
        with_deps("app", "1.0-1", &["python>=3.11"]),
        simple("python", "3.10-1"),
        simple("python", "3.11-1"),
        simple("python", "3.12-1"),
    ]);

    let app_id = provider.pool.intern_name("app");
    provider
        .pool
        .intern_version_set(app_id, DepConstraint::parse("app"));

    let result = solve(provider, &["app"]).unwrap();
    assert!(result.contains(&"app-1.0-1".to_string()));
    // Should pick 3.12 (highest matching >=3.11)
    assert!(result.contains(&"python-3.12-1".to_string()));
    assert!(!result.contains(&"python-3.10-1".to_string()));
}

#[test]
fn solve_shared_dependency() {
    // Both A and B depend on the same library
    let mut provider = build_provider(vec![
        with_deps("app-a", "1.0-1", &["libcommon"]),
        with_deps("app-b", "1.0-1", &["libcommon"]),
        simple("libcommon", "1.0-1"),
    ]);

    let a_id = provider.pool.intern_name("app-a");
    provider
        .pool
        .intern_version_set(a_id, DepConstraint::parse("app-a"));
    let b_id = provider.pool.intern_name("app-b");
    provider
        .pool
        .intern_version_set(b_id, DepConstraint::parse("app-b"));

    let result = solve(provider, &["app-a", "app-b"]).unwrap();
    assert_eq!(result.len(), 3);
    assert!(result.contains(&"libcommon-1.0-1".to_string()));
}

#[test]
fn solve_large_dependency_chain_at_scale() {
    // Synthetic stress test (roadmap #38): a 2000-package chain must resolve
    // to the full closure quickly and without unbounded growth.
    let count = 2000usize;
    let mut candidates = Vec::with_capacity(count);
    for index in 0..count {
        let name = format!("pkg{index:04}");
        if index == 0 {
            candidates.push(simple(&name, "1.0-1"));
        } else {
            let previous = format!("pkg{:04}", index - 1);
            candidates.push(with_deps(&name, "1.0-1", &[previous.as_str()]));
        }
    }

    let mut provider = build_provider(candidates);
    // The root has no dependents, so intern its requirement manually.
    let root_id = provider.pool.intern_name("pkg1999");
    provider
        .pool
        .intern_version_set(root_id, DepConstraint::parse("pkg1999"));

    let started = std::time::Instant::now();
    let solved = solve(provider, &["pkg1999"]).expect("large closure must resolve");
    let elapsed = started.elapsed();

    assert_eq!(solved.len(), count, "every chain link must be selected");
    assert!(solved.contains(&"pkg0000-1.0-1".to_string()));
    assert!(solved.contains(&"pkg1999-1.0-1".to_string()));
    assert!(
        elapsed.as_secs() < 30,
        "resolver took too long: {elapsed:?}"
    );
}

#[test]
fn solve_wide_fanout_resolves_all_versions_once() {
    // One root package depending on 500 distinct libraries.
    let mut deps: Vec<String> = (0..500).map(|i| format!("lib{i:03}")).collect();
    deps.sort();
    let dep_refs: Vec<&str> = deps.iter().map(String::as_str).collect();

    let mut candidates = vec![with_deps("app-wide", "1.0-1", &dep_refs)];
    for name in &deps {
        candidates.push(simple(name, "1.0-1"));
    }

    let mut provider = build_provider(candidates);
    let root_id = provider.pool.intern_name("app-wide");
    provider
        .pool
        .intern_version_set(root_id, DepConstraint::parse("app-wide"));

    let solved = solve(provider, &["app-wide"]).expect("fan-out must resolve");
    assert_eq!(solved.len(), 501);
}
