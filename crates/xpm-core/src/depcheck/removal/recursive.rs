//! Reverse-dependency discovery and recursive selection.

use super::*;

pub(super) fn find_broken(
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

pub(super) fn select_recursive(
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
