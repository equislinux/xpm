//! Solution conflict checks against installed packages.

use super::*;

/// Return the first installed package that `candidate` conflicts with, unless
/// the installed package is explicitly requested in the same transaction or
/// is replaced by the candidate.
pub(super) fn conflict_with_installed(
    candidate: &PackageCandidate,
    installed: &[PackageCandidate],
    explicit_names: &HashSet<&str>,
) -> Option<(String, DepConstraint)> {
    for package in installed {
        if package.name == candidate.name
            || explicit_names.contains(package.name.as_str())
            || candidate
                .replaces
                .iter()
                .any(|replaces| package_satisfies(package, replaces))
        {
            continue;
        }
        if let Some(conflict) = candidate
            .conflicts
            .iter()
            .find(|conflict| package_satisfies(package, conflict))
        {
            return Some((package.name.clone(), conflict.clone()));
        }
    }
    None
}

/// Reject solutions where two packages that will be installed conflict with
/// each other, or where a new package conflicts with a package that stays
/// installed. Self-conflicting `provides`/`conflicts` pairs (e.g. systemd and
/// udev) are skipped because ALPM excludes a package from its own conflict.
pub(super) fn check_solution_conflicts(
    solved: &[usize],
    solved_set: &HashSet<usize>,
    snapshot: &[PackageCandidate],
    installed_indexes: &HashSet<usize>,
) -> XpmResult<()> {
    for &index in solved {
        if installed_indexes.contains(&index) {
            continue;
        }
        let candidate = &snapshot[index];

        for &other_index in solved {
            if other_index == index {
                continue;
            }
            let other = &snapshot[other_index];
            if other.name == candidate.name {
                continue;
            }
            if let Some(conflict) = candidate
                .conflicts
                .iter()
                .find(|conflict| package_satisfies(other, conflict))
            {
                return Err(XpmError::DependencyConflict(format!(
                    "installing {} conflicts with {} ('{}')",
                    candidate.name, other.name, conflict
                )));
            }
        }

        for (other_index, other) in snapshot.iter().enumerate() {
            if !installed_indexes.contains(&other_index)
                || solved_set.contains(&other_index)
                || other.name == candidate.name
            {
                continue;
            }
            if let Some(conflict) = candidate
                .conflicts
                .iter()
                .find(|conflict| package_satisfies(other, conflict))
            {
                return Err(XpmError::DependencyConflict(format!(
                    "installing {} conflicts with installed package {} ('{}')",
                    candidate.name, other.name, conflict
                )));
            }
        }
    }

    Ok(())
}
