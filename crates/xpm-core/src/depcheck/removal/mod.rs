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

mod order;
mod recursive;

use order::sort_removal;
use recursive::{find_broken, select_recursive};
