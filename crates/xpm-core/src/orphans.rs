//! Orphan detection over the installed package set.
//!
//! A dependency package (`reason: dep`) is an orphan when it is not reachable
//! from any explicitly installed package through the recorded dependency
//! edges. Legacy entries without a `depends` record are never reported: their
//! edges are unknown, so listing them could suggest removing something that is
//! still required.

use std::collections::{HashMap, HashSet, VecDeque};

/// Installed package view needed to compute orphans.
#[derive(Debug, Clone, Default)]
pub struct InstalledPackage {
    pub name: String,
    pub explicit: bool,
    /// Declared runtime dependencies (raw specs such as `libc>=2.39`).
    pub depends: Vec<String>,
    /// Virtual names the package provides.
    pub provides: Vec<String>,
    /// `false` for legacy entries without a `depends` record (never reported).
    pub has_depends_record: bool,
}

/// Strips version constraints from a dependency spec (`libc>=2.39` -> `libc`).
pub fn dep_name(spec: &str) -> &str {
    let spec = spec.trim();
    let cut = spec.find(&['<', '>', '='][..]).unwrap_or(spec.len());
    spec[..cut].trim()
}

/// Returns the sorted names of installed dependency packages that no
/// explicitly installed package requires (directly or transitively).
pub fn find_orphans(packages: &[InstalledPackage]) -> Vec<String> {
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    let mut providers: HashMap<&str, Vec<usize>> = HashMap::new();

    for (index, package) in packages.iter().enumerate() {
        by_name.insert(package.name.as_str(), index);
        for provide in &package.provides {
            providers.entry(dep_name(provide)).or_default().push(index);
        }
    }

    let mut reachable: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (index, package) in packages.iter().enumerate() {
        if package.explicit {
            reachable.insert(index);
            queue.push_back(index);
        }
    }

    while let Some(index) = queue.pop_front() {
        for dep in &packages[index].depends {
            let name = dep_name(dep);
            if let Some(&candidate) = by_name.get(name) {
                if reachable.insert(candidate) {
                    queue.push_back(candidate);
                }
            }
            if let Some(satisfied_by) = providers.get(name) {
                for &candidate in satisfied_by {
                    if reachable.insert(candidate) {
                        queue.push_back(candidate);
                    }
                }
            }
        }
    }

    let mut orphans: Vec<String> = packages
        .iter()
        .enumerate()
        .filter(|(index, package)| {
            !package.explicit && package.has_depends_record && !reachable.contains(index)
        })
        .map(|(_, package)| package.name.clone())
        .collect();
    orphans.sort();
    orphans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, explicit: bool, depends: &[&str], provides: &[&str]) -> InstalledPackage {
        InstalledPackage {
            name: name.to_string(),
            explicit,
            depends: depends.iter().map(|d| d.to_string()).collect(),
            provides: provides.iter().map(|p| p.to_string()).collect(),
            has_depends_record: true,
        }
    }

    #[test]
    fn explicit_package_keeps_direct_and_transitive_dependencies() {
        let packages = vec![
            pkg("app", true, &["liba"], &[]),
            pkg("liba", false, &["libb"], &[]),
            pkg("libb", false, &[], &[]),
        ];

        assert!(find_orphans(&packages).is_empty());
    }

    #[test]
    fn unneeded_dependency_is_an_orphan() {
        let packages = vec![
            pkg("app", true, &[], &[]),
            pkg("liba", false, &[], &[]),
            pkg("libb", false, &[], &[]),
        ];

        assert_eq!(find_orphans(&packages), vec!["liba", "libb"]);
    }

    #[test]
    fn version_constraints_are_stripped_when_matching() {
        let packages = vec![
            pkg("app", true, &["liba>=1.2"], &[]),
            pkg("liba", false, &[], &[]),
        ];

        assert!(find_orphans(&packages).is_empty());
    }

    #[test]
    fn provides_satisfy_dependencies() {
        let packages = vec![
            pkg("app", true, &["browser"], &[]),
            pkg("firefox", false, &[], &["browser=128.0"]),
        ];

        assert!(find_orphans(&packages).is_empty());
    }

    #[test]
    fn legacy_entries_without_record_are_never_reported() {
        let packages = vec![
            pkg("app", true, &[], &[]),
            InstalledPackage {
                name: "libold".to_string(),
                explicit: false,
                depends: Vec::new(),
                provides: Vec::new(),
                has_depends_record: false,
            },
        ];

        assert!(find_orphans(&packages).is_empty());
    }

    #[test]
    fn recorded_empty_depends_can_be_orphan() {
        let packages = vec![pkg("liba", false, &[], &[])];

        assert_eq!(find_orphans(&packages), vec!["liba"]);
    }

    #[test]
    fn cycles_are_fine_and_explicit_roots_are_never_orphans() {
        let packages = vec![pkg("a", true, &["b"], &[]), pkg("b", false, &["a"], &[])];

        assert!(find_orphans(&packages).is_empty());
    }

    #[test]
    fn dep_name_strips_operators_and_whitespace() {
        assert_eq!(dep_name("libc>=2.39"), "libc");
        assert_eq!(dep_name("libc<3"), "libc");
        assert_eq!(dep_name("libc=2.39"), "libc");
        assert_eq!(dep_name(" libc "), "libc");
    }
}
