//! Install-time dependency and conflict checks.

use super::*;

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

/// Verify that every package remaining after `removed` (plus `added`) has all
/// of its dependencies satisfied by the resulting set.
///
/// This catches reverse-dependency breakage during upgrades and replacements,
/// not just the dependencies of the transaction targets.
pub fn check_installed_closure(
    installed: &[InstalledPkg],
    removed: &[String],
    added: &[InstalledPkg],
) -> XpmResult<()> {
    let removed: HashSet<&str> = removed.iter().map(String::as_str).collect();
    let live: Vec<&InstalledPkg> = installed
        .iter()
        .filter(|package| !removed.contains(package.name.as_str()))
        .collect();
    let candidates: Vec<&InstalledPkg> = live.iter().copied().chain(added.iter()).collect();

    let mut problems = Vec::new();
    for package in &candidates {
        for dependency in &package.depends {
            if candidates
                .iter()
                .any(|candidate| satisfies(candidate, dependency, false))
            {
                continue;
            }

            let causing = installed.iter().find(|candidate| {
                removed.contains(candidate.name.as_str()) && satisfies(candidate, dependency, false)
            });

            match causing {
                Some(causing) => problems.push(format!(
                    "removing {} breaks dependency '{}' required by {}",
                    causing.name, dependency, package.name
                )),
                None => problems.push(format!(
                    "unable to satisfy dependency '{}' required by {}",
                    dependency, package.name
                )),
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
