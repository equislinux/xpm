//! Dependency satisfaction checks used by `xpm deptest` (`pacman -T`).
//!
//! A dependency is satisfied by an installed package when either its name and
//! version match, or when the package provides the dependency name (with a
//! versioned provide when the dependency itself is versioned, matching ALPM's
//! `foo=1.2` provide semantics).

use crate::resolver::{DepConstraint, Operator, Version};

/// An installed package as seen by the dependency tester.
#[derive(Debug, Clone)]
pub struct InstalledDependency {
    pub name: String,
    pub version: Version,
    pub provides: Vec<DepConstraint>,
}

impl InstalledDependency {
    /// Builds the view from the local-database records.
    pub fn new(name: String, version: &str, provides: &[String]) -> Self {
        Self {
            name,
            version: Version::parse(version),
            provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
        }
    }

    /// Whether this package satisfies `dependency`.
    pub fn satisfies(&self, dependency: &DepConstraint) -> bool {
        if self.name == dependency.name && dependency.matches(&self.version) {
            return true;
        }

        for provide in &self.provides {
            if provide.name != dependency.name {
                continue;
            }

            if dependency.op.is_none() {
                return true;
            }

            // ALPM only supports versioned provides in the `name=version` form.
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
}

/// Returns the dependency specs that are not satisfied by any installed
/// package, preserving the input order.
pub fn unmet_dependencies(specs: &[String], installed: &[InstalledDependency]) -> Vec<String> {
    specs
        .iter()
        .filter(|spec| {
            let dependency = DepConstraint::parse(spec.trim());
            !installed
                .iter()
                .any(|package| package.satisfies(&dependency))
        })
        .map(|spec| spec.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(name: &str, version: &str, provides: &[&str]) -> InstalledDependency {
        InstalledDependency::new(
            name.to_string(),
            version,
            &provides.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn name_and_version_satisfy() {
        let packages = vec![installed("glibc", "2.39-1", &[])];

        assert!(unmet_dependencies(&["glibc".to_string()], &packages).is_empty());
        assert!(unmet_dependencies(&["glibc>=2.38".to_string()], &packages).is_empty());
        assert!(unmet_dependencies(&["glibc=2.39-1".to_string()], &packages).is_empty());
        assert_eq!(
            unmet_dependencies(&["glibc>2.40".to_string()], &packages),
            vec!["glibc>2.40".to_string()]
        );
    }

    #[test]
    fn versioned_provides_satisfy_matching_constraints() {
        let packages = vec![installed("firefox", "128.0-1", &["browser=128.0"])];

        assert!(unmet_dependencies(&["browser".to_string()], &packages).is_empty());
        assert!(unmet_dependencies(&["browser>=120".to_string()], &packages).is_empty());
        assert_eq!(
            unmet_dependencies(&["browser>=130".to_string()], &packages),
            vec!["browser>=130".to_string()]
        );
    }

    #[test]
    fn unversioned_provides_do_not_satisfy_versioned_dependencies() {
        let packages = vec![installed("firefox", "128.0-1", &["browser"])];

        assert!(unmet_dependencies(&["browser".to_string()], &packages).is_empty());
        assert_eq!(
            unmet_dependencies(&["browser>=120".to_string()], &packages),
            vec!["browser>=120".to_string()]
        );
    }

    #[test]
    fn unmet_specs_preserve_order_and_duplicates() {
        let packages = vec![installed("glibc", "2.39-1", &[])];
        let specs = vec![
            "missing>=1".to_string(),
            "glibc".to_string(),
            "missing>=1".to_string(),
        ];

        assert_eq!(
            unmet_dependencies(&specs, &packages),
            vec!["missing>=1".to_string(), "missing>=1".to_string()]
        );
    }

    #[test]
    fn empty_installed_set_reports_every_spec() {
        let specs = vec!["bash".to_string(), "zlib>=1".to_string()];
        assert_eq!(unmet_dependencies(&specs, &[]), specs);
    }
}
