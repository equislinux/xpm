//! Removal planning: reverse dependencies, recursive/cascade modes and ordering.

use super::*;

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

pub(super) fn satisfies(
    package: &InstalledPkg,
    dependency: &DepConstraint,
    ignore_versions: bool,
) -> bool {
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
