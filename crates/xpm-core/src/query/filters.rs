//! Foreign/unrequired package filters (`-Qm`, `-Qt`).

use super::*;

/// Installed packages whose name is absent from every configured sync DB.
pub fn find_foreign(
    packages: &[LocalPackage],
    sync_names: &HashSet<String>,
) -> Vec<(String, String)> {
    let mut foreign: Vec<(String, String)> = packages
        .iter()
        .filter(|package| !sync_names.contains(&package.name))
        .map(|package| (package.name.clone(), package.version.clone()))
        .collect();
    foreign.sort();
    foreign
}

/// Installed packages that no *other* installed package depends on directly.
///
/// Only direct `depends` edges are considered (never optdepends or transitive
/// reachability). Versioned provides are honoured. When `deps_only` is true,
/// explicitly installed packages are omitted (pacman `-Qdt`).
pub fn find_unrequired(packages: &[LocalPackage], deps_only: bool) -> Vec<String> {
    let installed: Vec<InstalledDependency> =
        packages.iter().map(LocalPackage::dependency_view).collect();

    let mut required = HashSet::new();
    for (consumer_index, consumer) in packages.iter().enumerate() {
        for spec in &consumer.depends {
            let dependency = DepConstraint::parse(spec.trim());
            for (candidate_index, candidate) in installed.iter().enumerate() {
                if candidate_index == consumer_index {
                    continue;
                }
                if candidate.satisfies(&dependency) {
                    required.insert(candidate.name.clone());
                }
            }
        }
    }

    let mut unrequired: Vec<String> = packages
        .iter()
        .filter(|package| !required.contains(&package.name))
        .filter(|package| !deps_only || package.reason == InstallReason::Dep)
        .map(|package| package.name.clone())
        .collect();
    unrequired.sort();
    unrequired.dedup();
    unrequired
}
