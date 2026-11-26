//! Replacement selection.

use super::*;

/// A package currently installed in the local database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledReplacement {
    pub name: String,
    pub version: Version,
    pub reason: InstallReason,
    /// Groups recorded for the installed package, used by `IgnoreGroup`.
    pub groups: Vec<String>,
}

/// A repository package that may replace an installed package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacementCandidate {
    pub name: String,
    pub version: Version,
    /// Repository priority; lower ranks are higher priority.
    pub repo_rank: usize,
    /// Literal name and version constraints from `replaces`.
    pub replaces: Vec<DepConstraint>,
    pub provides: Vec<DepConstraint>,
    pub conflicts: Vec<DepConstraint>,
}

/// A selected replacement. The old package's reason is copied to the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub old_name: String,
    pub old_version: Version,
    pub old_reason: InstallReason,
    pub new_name: String,
    pub new_version: Version,
    pub repo_rank: usize,
}

/// Selects replacements for installed packages during a sysupgrade.
///
/// Repositories are visited in ascending [`ReplacementCandidate::repo_rank`]
/// order. Within one repository every candidate whose `replaces` literally
/// matches the installed name and version is selected unless ignored; if any
/// remain, lower-priority repositories are not consulted. Otherwise the first
/// repository containing a literal same-name package stops the scan, leaving
/// normal upgrade handling to the caller.
///
/// `IgnorePkg` patterns match both the installed and candidate names;
/// `IgnoreGroup` patterns match the installed package's groups.
/// [`ReplacementCandidate`] does not carry groups, so callers that need to
/// ignore candidate groups must pre-filter the candidate list.
///
/// This function performs no dependency, conflict, or file checks.
pub fn find_replacements(
    installed: &[InstalledReplacement],
    candidates: &[ReplacementCandidate],
    ignore_pkg: &[String],
    ignore_group: &[String],
) -> Vec<Replacement> {
    let ignore_pkg = IgnorePatterns::new(ignore_pkg);
    let ignore_group = IgnorePatterns::new(ignore_group);

    let mut repo_ranks: Vec<usize> = candidates
        .iter()
        .map(|candidate| candidate.repo_rank)
        .collect();
    repo_ranks.sort_unstable();
    repo_ranks.dedup();

    let mut replacements = Vec::new();

    for old in installed {
        if is_ignored(&old.name, &old.groups, &ignore_pkg, &ignore_group) {
            continue;
        }

        for rank in &repo_ranks {
            let mut accepted = Vec::new();
            let mut literal = false;

            for candidate in candidates.iter().filter(|c| c.repo_rank == *rank) {
                if candidate.name == old.name {
                    literal = true;
                }
                if is_ignored(&candidate.name, &[], &ignore_pkg, &ignore_group) {
                    continue;
                }
                if candidate
                    .replaces
                    .iter()
                    .any(|replace| replace.name == old.name && replace.matches(&old.version))
                {
                    accepted.push(candidate);
                }
            }

            if !accepted.is_empty() {
                replacements.extend(accepted.into_iter().map(|candidate| Replacement {
                    old_name: old.name.clone(),
                    old_version: old.version.clone(),
                    old_reason: old.reason,
                    new_name: candidate.name.clone(),
                    new_version: candidate.version.clone(),
                    repo_rank: candidate.repo_rank,
                }));
                break;
            }

            if literal {
                break;
            }
        }
    }

    replacements
}
