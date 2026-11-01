//! Removal planning with libalpm-compatible dependency checks.
//!
//! Implements the reverse dependency checks, recursive and cascade modes,
//! and reverse dependency ordering used by `pacman -R`.

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

#[derive(Debug)]
struct BrokenDep {
    target: String,
    depend: DepConstraint,
    causing: String,
}

#[derive(Debug, Default)]
struct RemovalSet {
    order: Vec<String>,
    names: HashSet<String>,
    selected: HashSet<String>,
}

impl RemovalSet {
    fn from_targets(targets: Vec<String>) -> Self {
        let mut set = Self::default();
        for target in targets {
            set.insert(&target);
        }
        set
    }

    fn insert(&mut self, name: &str) -> bool {
        if self.names.insert(name.to_string()) {
            self.order.push(name.to_string());
            true
        } else {
            false
        }
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    fn remove(&mut self, name: &str) -> bool {
        self.names.remove(name)
    }

    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Plans a removal transaction, returning packages in removal order.
pub fn plan_removal(
    installed: &[InstalledPkg],
    targets: &[String],
    options: &RemoveOptions,
) -> XpmResult<Vec<String>> {
    let by_name: HashMap<&str, &InstalledPkg> = installed
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    let mut seen = HashSet::new();
    let mut explicit_targets = Vec::new();

    for target in targets {
        if !seen.insert(target.as_str()) {
            continue;
        }
        if !by_name.contains_key(target.as_str()) {
            return Err(XpmError::Package(format!("target not found: {target}")));
        }
        explicit_targets.push(target.clone());
    }

    let mut removal = RemovalSet::from_targets(explicit_targets);

    if options.cascade && !options.skip_deps {
        loop {
            let broken = find_broken(installed, &removal.names, options.ignore_versions);
            if broken.is_empty() {
                break;
            }

            let mut added = false;
            for missing in &broken {
                added |= removal.insert(&missing.target);
            }
            if !added {
                break;
            }
        }
    }

    if options.recursive {
        select_recursive(
            installed,
            &mut removal,
            options.recursive_all,
            options.ignore_versions,
        );
    }

    if options.unneeded && !options.skip_deps {
        loop {
            if removal.is_empty() {
                return Ok(Vec::new());
            }

            let broken = find_broken(installed, &removal.names, options.ignore_versions);
            if broken.is_empty() {
                break;
            }

            let mut changed = false;
            for missing in broken {
                changed |= removal.remove(&missing.causing);
            }
            if !changed {
                break;
            }
        }
    }

    if !options.skip_deps {
        let broken = find_broken(installed, &removal.names, options.ignore_versions);
        if !broken.is_empty() {
            let message = broken
                .iter()
                .map(|missing| {
                    format!(
                        "removing {} breaks dependency '{}' required by {}",
                        missing.causing, missing.depend, missing.target
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            return Err(XpmError::DependencyConflict(message));
        }
    }

    let remaining: Vec<&str> = removal
        .order
        .iter()
        .filter(|name| removal.names.contains(name.as_str()))
        .map(String::as_str)
        .collect();

    Ok(sort_removal(installed, &remaining, options.ignore_versions))
}

fn satisfies(package: &InstalledPkg, dependency: &DepConstraint, ignore_versions: bool) -> bool {
    let versioned = !ignore_versions && dependency.op.is_some();

    if package.name == dependency.name && (!versioned || dependency.matches(&package.version)) {
        return true;
    }

    for provide in &package.provides {
        if provide.name != dependency.name {
            continue;
        }
        if !versioned {
            return true;
        }
        if provide.op == Some(Operator::Eq) {
            if let Some(version) = &provide.version {
                if dependency.matches(version) {
                    return true;
                }
            }
        }
    }

    false
}

fn find_broken(
    installed: &[InstalledPkg],
    removal: &HashSet<String>,
    ignore_versions: bool,
) -> Vec<BrokenDep> {
    let mut broken = Vec::new();

    for package in installed {
        if removal.contains(&package.name) {
            continue;
        }

        for dependency in &package.depends {
            let causing = installed.iter().find(|candidate| {
                removal.contains(&candidate.name)
                    && satisfies(candidate, dependency, ignore_versions)
            });
            let Some(causing) = causing else {
                continue;
            };

            let still_satisfied = installed.iter().any(|candidate| {
                !removal.contains(&candidate.name)
                    && satisfies(candidate, dependency, ignore_versions)
            });
            if still_satisfied {
                continue;
            }

            broken.push(BrokenDep {
                target: package.name.clone(),
                depend: dependency.clone(),
                causing: causing.name.clone(),
            });
        }
    }

    broken
}

fn select_recursive(
    installed: &[InstalledPkg],
    removal: &mut RemovalSet,
    include_explicit: bool,
    ignore_versions: bool,
) {
    let mut keep: HashSet<String> = installed
        .iter()
        .filter(|package| !removal.contains(&package.name))
        .map(|package| package.name.clone())
        .collect();

    let mut cursor = 0;
    while cursor < removal.order.len() {
        let source_name = removal.order[cursor].clone();
        cursor += 1;

        let Some(source) = installed.iter().find(|package| package.name == source_name) else {
            continue;
        };

        for candidate in installed {
            if !keep.contains(&candidate.name) {
                continue;
            }
            if !include_explicit && candidate.reason != InstallReason::Dep {
                continue;
            }
            if source
                .depends
                .iter()
                .any(|dependency| satisfies(candidate, dependency, ignore_versions))
            {
                keep.remove(&candidate.name);
                removal.insert(&candidate.name);
                removal.selected.insert(candidate.name.clone());
            }
        }
    }

    let mut keep_order: Vec<String> = installed
        .iter()
        .filter(|package| keep.contains(&package.name))
        .map(|package| package.name.clone())
        .collect();

    let mut index = 0;
    while index < keep_order.len() {
        let keeper_name = keep_order[index].clone();
        index += 1;

        let Some(keeper) = installed.iter().find(|package| package.name == keeper_name) else {
            continue;
        };

        for candidate in installed {
            if !removal.selected.contains(&candidate.name) {
                continue;
            }
            if keeper
                .depends
                .iter()
                .any(|dependency| satisfies(candidate, dependency, ignore_versions))
            {
                removal.selected.remove(&candidate.name);
                removal.remove(&candidate.name);
                if keep.insert(candidate.name.clone()) {
                    keep_order.push(candidate.name.clone());
                }
            }
        }
    }
}

fn sort_removal(installed: &[InstalledPkg], names: &[&str], ignore_versions: bool) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }

    let by_name: HashMap<&str, &InstalledPkg> = installed
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    let edges: Vec<Vec<usize>> = names
        .iter()
        .map(|name| {
            let package = by_name.get(name).copied();
            names
                .iter()
                .enumerate()
                .filter_map(|(index, candidate)| {
                    let candidate = by_name.get(candidate).copied()?;
                    let package = package?;
                    package
                        .depends
                        .iter()
                        .any(|dependency| satisfies(candidate, dependency, ignore_versions))
                        .then_some(index)
                })
                .collect()
        })
        .collect();

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Unvisited,
        Processing,
        Done,
    }

    let mut state = vec![State::Unvisited; names.len()];
    let mut post_order = Vec::new();

    for root in 0..names.len() {
        if state[root] != State::Unvisited {
            continue;
        }

        let mut stack = vec![(root, 0usize)];
        state[root] = State::Processing;

        while let Some(&(node, next)) = stack.last() {
            if next < edges[node].len() {
                let child = edges[node][next];
                stack.last_mut().expect("stack is not empty").1 += 1;

                if state[child] == State::Unvisited {
                    state[child] = State::Processing;
                    stack.push((child, 0));
                }
            } else {
                state[node] = State::Done;
                post_order.push(names[node].to_string());
                stack.pop();
            }
        }
    }

    post_order.reverse();
    post_order
}

#[cfg(test)]
mod install_checks {
    use super::*;

    fn installed(name: &str, version: &str, provides: &[&str]) -> InstalledPkg {
        InstalledPkg {
            name: name.to_string(),
            version: Version::parse(version),
            depends: vec![],
            provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
            reason: InstallReason::Explicit,
        }
    }

    fn target(
        name: &str,
        version: &str,
        depends: &[&str],
        provides: &[&str],
        conflicts: &[&str],
    ) -> InstallTarget {
        InstallTarget {
            name: name.to_string(),
            version: Version::parse(version),
            depends: depends.iter().map(|d| DepConstraint::parse(d)).collect(),
            provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
            conflicts: conflicts.iter().map(|c| DepConstraint::parse(c)).collect(),
        }
    }

    #[test]
    fn local_target_missing_dependency_is_reported() {
        let err = check_install(&[], &[target("a", "1-1", &["b"], &[], &[])]).unwrap_err();
        assert!(matches!(err, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn installed_package_satisfies_local_dependency() {
        check_install(
            &[installed("b", "1-1", &["virt"])],
            &[target("a", "1-1", &["b"], &[], &[])],
        )
        .expect("installed b satisfies a");
    }

    #[test]
    fn target_satisfies_another_target_dependency() {
        check_install(
            &[],
            &[
                target("a", "1-1", &["b"], &[], &[]),
                target("b", "1-1", &[], &["virt"], &[]),
            ],
        )
        .expect("target b satisfies a");
    }

    #[test]
    fn versioned_dependency_requires_versioned_provide() {
        let err = check_install(
            &[installed("b", "1-1", &["virt"])],
            &[target("a", "1-1", &["virt>=2"], &[], &[])],
        )
        .unwrap_err();
        assert!(matches!(err, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn target_conflict_with_installed_is_reported() {
        let err = check_install(
            &[installed("b", "1-1", &[])],
            &[target("a", "1-1", &[], &[], &["b"])],
        )
        .unwrap_err();
        assert!(matches!(err, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn target_conflict_with_other_target_is_reported() {
        let err = check_install(
            &[],
            &[
                target("a", "1-1", &[], &[], &["b"]),
                target("b", "1-1", &[], &[], &[]),
            ],
        )
        .unwrap_err();
        assert!(matches!(err, XpmError::DependencyConflict(_)));
    }

    #[test]
    fn self_provide_conflict_is_ignored() {
        check_install(&[], &[target("udev", "1-1", &[], &["udev"], &["udev"])])
            .expect("a package does not conflict with itself");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(
        name: &str,
        version: &str,
        depends: &[&str],
        provides: &[&str],
        reason: InstallReason,
    ) -> InstalledPkg {
        InstalledPkg {
            name: name.to_string(),
            version: Version::parse(version),
            depends: depends
                .iter()
                .map(|dependency| DepConstraint::parse(dependency))
                .collect(),
            provides: provides
                .iter()
                .map(|provide| DepConstraint::parse(provide))
                .collect(),
            reason,
        }
    }

    fn explicit(name: &str, version: &str, depends: &[&str], provides: &[&str]) -> InstalledPkg {
        package(name, version, depends, provides, InstallReason::Explicit)
    }

    fn as_dep(name: &str, version: &str, depends: &[&str], provides: &[&str]) -> InstalledPkg {
        let mut package = package(name, version, depends, provides, InstallReason::Dep);
        package.reason = InstallReason::Dep;
        package
    }

    fn targets(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn expect_broken(result: XpmResult<Vec<String>>, message: &str) {
        match result {
            Err(XpmError::DependencyConflict(actual)) => assert_eq!(actual, message),
            other => panic!("expected dependency conflict, got {other:?}"),
        }
    }

    #[test]
    fn provider_is_checked_on_removal() {
        let installed = vec![
            as_dep("virtual-b", "1.0-1", &[], &["virt"]),
            explicit("needs-virt", "1.0-1", &["virt"], &[]),
        ];

        expect_broken(
            plan_removal(
                &installed,
                &targets(&["virtual-b"]),
                &RemoveOptions::default(),
            ),
            "removing virtual-b breaks dependency 'virt' required by needs-virt",
        );
    }

    #[test]
    fn remaining_provider_satisfies_dependency() {
        let installed = vec![
            as_dep("virtual-b", "1.0-1", &[], &["virt"]),
            as_dep("virtual-c", "1.0-1", &[], &["virt"]),
            explicit("needs-virt", "1.0-1", &["virt"], &[]),
        ];

        let plan = plan_removal(
            &installed,
            &targets(&["virtual-b"]),
            &RemoveOptions::default(),
        )
        .expect("removal succeeds");

        assert_eq!(plan, vec!["virtual-b"]);
    }

    #[test]
    fn versioned_provide_must_match_dependency() {
        let installed = vec![
            as_dep("libfoo", "1.0-1", &[], &["foo=3"]),
            explicit("app", "1.0-1", &["foo>=2"], &[]),
        ];

        expect_broken(
            plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default()),
            "removing libfoo breaks dependency 'foo>=2' required by app",
        );
    }

    #[test]
    fn versioned_provide_with_wrong_version_does_not_satisfy() {
        let installed = vec![
            as_dep("libfoo", "1.0-1", &[], &["foo=1"]),
            explicit("app", "1.0-1", &["foo>=2"], &[]),
        ];

        let plan = plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default())
            .expect("dependency was already broken");

        assert_eq!(plan, vec!["libfoo"]);
    }

    #[test]
    fn unversioned_provide_does_not_satisfy_versioned_dependency() {
        let installed = vec![
            as_dep("libfoo", "1.0-1", &[], &["foo"]),
            explicit("app", "1.0-1", &["foo>=2"], &[]),
        ];

        let plan = plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default())
            .expect("dependency was already broken");

        assert_eq!(plan, vec!["libfoo"]);
    }

    #[test]
    fn ignore_versions_still_checks_names() {
        let installed = vec![
            as_dep("libfoo", "1.0-1", &[], &[]),
            explicit("app", "1.0-1", &["libfoo>=2"], &[]),
        ];
        let options = RemoveOptions {
            ignore_versions: true,
            ..RemoveOptions::default()
        };

        expect_broken(
            plan_removal(&installed, &targets(&["libfoo"]), &options),
            "removing libfoo breaks dependency 'libfoo>=2' required by app",
        );
    }

    #[test]
    fn ignore_versions_matches_unversioned_provide() {
        let installed = vec![
            as_dep("libfoo", "1.0-1", &[], &["foo"]),
            explicit("app", "1.0-1", &["foo>=2"], &[]),
        ];
        let options = RemoveOptions {
            ignore_versions: true,
            ..RemoveOptions::default()
        };

        expect_broken(
            plan_removal(&installed, &targets(&["libfoo"]), &options),
            "removing libfoo breaks dependency 'foo>=2' required by app",
        );
    }

    #[test]
    fn skip_deps_ignores_broken_dependencies() {
        let installed = vec![
            as_dep("libfoo", "1.0-1", &[], &[]),
            explicit("app", "1.0-1", &["libfoo>=2"], &[]),
        ];
        let options = RemoveOptions {
            skip_deps: true,
            ..RemoveOptions::default()
        };

        let plan =
            plan_removal(&installed, &targets(&["libfoo"]), &options).expect("removal succeeds");

        assert_eq!(plan, vec!["libfoo"]);
    }

    #[test]
    fn already_broken_dependency_is_ignored() {
        let installed = vec![
            as_dep("libfoo", "1.0-1", &[], &[]),
            explicit("app", "1.0-1", &["libfoo>=2"], &[]),
        ];

        let plan = plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default())
            .expect("removal succeeds");

        assert_eq!(plan, vec!["libfoo"]);
    }

    #[test]
    fn recursive_removes_chain() {
        let installed = vec![
            explicit("a", "1.0-1", &["b"], &[]),
            as_dep("b", "1.0-1", &["c"], &[]),
            as_dep("c", "1.0-1", &[], &[]),
        ];
        let options = RemoveOptions {
            recursive: true,
            ..RemoveOptions::default()
        };

        let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");

        assert_eq!(plan, vec!["a", "b", "c"]);
    }

    #[test]
    fn recursive_keeps_explicit_dependency() {
        let installed = vec![
            explicit("a", "1.0-1", &["b"], &[]),
            as_dep("b", "1.0-1", &["c"], &[]),
            explicit("c", "1.0-1", &[], &[]),
        ];
        let options = RemoveOptions {
            recursive: true,
            ..RemoveOptions::default()
        };

        let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");
        assert_eq!(plan, vec!["a", "b"]);

        let options = RemoveOptions {
            recursive: true,
            recursive_all: true,
            ..RemoveOptions::default()
        };
        let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");
        assert_eq!(plan, vec!["a", "b", "c"]);
    }

    #[test]
    fn recursive_keeps_shared_dependency_and_its_dependencies() {
        let installed = vec![
            explicit("a", "1.0-1", &["b"], &[]),
            as_dep("b", "1.0-1", &["c"], &[]),
            as_dep("c", "1.0-1", &[], &[]),
            explicit("e", "1.0-1", &["b"], &[]),
        ];
        let options = RemoveOptions {
            recursive: true,
            ..RemoveOptions::default()
        };

        let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");

        assert_eq!(plan, vec!["a"]);
    }

    #[test]
    fn recursive_cycle_terminates() {
        let installed = vec![
            as_dep("a", "1.0-1", &["b"], &[]),
            as_dep("b", "1.0-1", &["a"], &[]),
        ];
        let options = RemoveOptions {
            recursive: true,
            ..RemoveOptions::default()
        };

        let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");

        assert_eq!(plan, vec!["a", "b"]);
    }

    #[test]
    fn cascade_removes_dependents() {
        let installed = vec![
            explicit("a", "1.0-1", &["b"], &[]),
            as_dep("b", "1.0-1", &[], &[]),
        ];
        let options = RemoveOptions {
            cascade: true,
            ..RemoveOptions::default()
        };

        let plan = plan_removal(&installed, &targets(&["b"]), &options).expect("removal succeeds");

        assert_eq!(plan, vec!["a", "b"]);
    }

    #[test]
    fn cascade_then_recursive_removes_chain() {
        let installed = vec![
            explicit("a", "1.0-1", &["b"], &[]),
            as_dep("b", "1.0-1", &["c"], &[]),
            as_dep("c", "1.0-1", &[], &[]),
        ];
        let options = RemoveOptions {
            cascade: true,
            recursive: true,
            ..RemoveOptions::default()
        };

        let plan = plan_removal(&installed, &targets(&["b"]), &options).expect("removal succeeds");

        assert_eq!(plan, vec!["a", "b", "c"]);
    }

    #[test]
    fn unneeded_keeps_required_target() {
        let installed = vec![
            as_dep("a", "1.0-1", &[], &[]),
            as_dep("b", "1.0-1", &[], &[]),
            explicit("c", "1.0-1", &["a"], &[]),
        ];
        let options = RemoveOptions {
            unneeded: true,
            ..RemoveOptions::default()
        };

        let plan =
            plan_removal(&installed, &targets(&["a", "b"]), &options).expect("removal succeeds");

        assert_eq!(plan, vec!["b"]);
    }

    #[test]
    fn unneeded_with_all_targets_required_is_empty() {
        let installed = vec![
            as_dep("a", "1.0-1", &[], &[]),
            as_dep("b", "1.0-1", &[], &[]),
            explicit("c", "1.0-1", &["a"], &[]),
            explicit("d", "1.0-1", &["b"], &[]),
        ];
        let options = RemoveOptions {
            unneeded: true,
            ..RemoveOptions::default()
        };

        let plan =
            plan_removal(&installed, &targets(&["a", "b"]), &options).expect("removal succeeds");

        assert!(plan.is_empty());
    }

    #[test]
    fn target_and_its_dependency_can_be_removed_together() {
        let installed = vec![
            explicit("app", "1.0-1", &["libfoo>=2"], &[]),
            as_dep("libfoo", "2.0-1", &[], &[]),
        ];

        let plan = plan_removal(
            &installed,
            &targets(&["app", "libfoo"]),
            &RemoveOptions::default(),
        )
        .expect("removal succeeds");

        assert_eq!(plan, vec!["app", "libfoo"]);
    }

    #[test]
    fn unknown_target_is_an_error() {
        let installed = vec![explicit("a", "1.0-1", &[], &[])];

        let error = plan_removal(
            &installed,
            &targets(&["missing"]),
            &RemoveOptions::default(),
        )
        .expect_err("unknown target");

        match error {
            XpmError::Package(message) => assert_eq!(message, "target not found: missing"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn duplicate_targets_are_removed_once() {
        let installed = vec![explicit("a", "1.0-1", &[], &[])];

        let plan = plan_removal(
            &installed,
            &targets(&["a", "a", "a"]),
            &RemoveOptions::default(),
        )
        .expect("removal succeeds");

        assert_eq!(plan, vec!["a"]);
    }

    #[test]
    fn multiple_broken_dependencies_are_joined_with_newlines() {
        let installed = vec![
            as_dep("a", "1.0-1", &[], &[]),
            as_dep("b", "1.0-1", &[], &[]),
            explicit("c", "1.0-1", &["a"], &[]),
            explicit("d", "1.0-1", &["b"], &[]),
        ];

        expect_broken(
            plan_removal(
                &installed,
                &targets(&["a", "b"]),
                &RemoveOptions::default(),
            ),
            "removing a breaks dependency 'a' required by c\nremoving b breaks dependency 'b' required by d",
        );
    }

    #[test]
    fn self_provided_dependency_is_not_broken() {
        let installed = vec![
            as_dep("a", "1.0-1", &[], &[]),
            explicit("c", "1.0-1", &["virt"], &["virt=1"]),
        ];

        let plan = plan_removal(&installed, &targets(&["a"]), &RemoveOptions::default())
            .expect("removal succeeds");

        assert_eq!(plan, vec!["a"]);
    }

    #[test]
    fn sorted_dependents_before_dependencies() {
        let installed = vec![
            as_dep("z-dependency", "1.0-1", &[], &[]),
            explicit("a-dependent", "1.0-1", &["z-dependency"], &[]),
            explicit("m-dependent", "1.0-1", &["z-dependency"], &[]),
        ];

        let plan = plan_removal(
            &installed,
            &targets(&["a-dependent", "m-dependent", "z-dependency"]),
            &RemoveOptions::default(),
        )
        .expect("removal succeeds");

        assert_eq!(plan, vec!["m-dependent", "a-dependent", "z-dependency"]);
    }
}

/// A package being installed, used for install-time dependency/conflict checks.
#[derive(Debug, Clone)]
pub struct InstallTarget {
    pub name: String,
    pub version: Version,
    pub depends: Vec<DepConstraint>,
    pub provides: Vec<DepConstraint>,
    pub conflicts: Vec<DepConstraint>,
}

/// Check that every target dependency is satisfied by the installed set or
/// another target, and that target conflicts do not hit installed or other
/// target packages.
///
/// Packages in `targets` are excluded from the installed set because their new
/// version replaces the installed one.
pub fn check_install(installed: &[InstalledPkg], targets: &[InstallTarget]) -> XpmResult<()> {
    let target_names: HashSet<&str> = targets.iter().map(|target| target.name.as_str()).collect();
    let installed_live: Vec<&InstalledPkg> = installed
        .iter()
        .filter(|package| !target_names.contains(package.name.as_str()))
        .collect();
    let target_pkgs: Vec<InstalledPkg> = targets
        .iter()
        .map(|target| InstalledPkg {
            name: target.name.clone(),
            version: target.version.clone(),
            depends: target.depends.clone(),
            provides: target.provides.clone(),
            reason: InstallReason::Explicit,
        })
        .collect();

    let mut problems = Vec::new();

    for target in targets {
        for dependency in &target.depends {
            let satisfied = installed_live
                .iter()
                .any(|package| satisfies(package, dependency, false))
                || target_pkgs
                    .iter()
                    .any(|package| satisfies(package, dependency, false));
            if !satisfied {
                problems.push(format!(
                    "installing {} requires '{}', which is not provided by any installed or target package",
                    target.name, dependency
                ));
            }
        }
    }

    for target in targets {
        for conflict in &target.conflicts {
            for package in &installed_live {
                if satisfies(package, conflict, false) {
                    problems.push(format!(
                        "installing {} conflicts with installed package {} ('{}')",
                        target.name, package.name, conflict
                    ));
                }
            }
            for other in &target_pkgs {
                if other.name == target.name {
                    continue;
                }
                if satisfies(other, conflict, false) {
                    problems.push(format!(
                        "installing {} conflicts with {} ('{}')",
                        target.name, other.name, conflict
                    ));
                }
            }
        }
    }

    if problems.is_empty() {
        return Ok(());
    }
    problems.sort();
    problems.dedup();
    Err(XpmError::DependencyConflict(problems.join("\n")))
}
