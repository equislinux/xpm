//! Sysupgrade replacement planning from `replaces` metadata.
//!
//! pacman consults `replaces` only during sysupgrade (`-Su`/`-Suu`), never for
//! a plain install of a new package. This module performs selection only: the
//! caller must add each old package to the transaction remove set and run
//! dependency, conflict, and file planning against `(local - remove) + add`.
//! `replaces` does not imply `conflicts` or `provides`.

use glob::{MatchOptions, Pattern};

use crate::install_reason::InstallReason;
use crate::resolver::{DepConstraint, Version};

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

#[derive(Debug, Clone)]
struct IgnorePattern {
    negated: bool,
    pattern: Pattern,
}

#[derive(Debug, Clone, Default)]
struct IgnorePatterns {
    patterns: Vec<IgnorePattern>,
}

impl IgnorePatterns {
    fn new(patterns: &[String]) -> Self {
        let patterns = patterns
            .iter()
            .filter_map(|raw| {
                let (negated, pattern) = if let Some(rest) = raw.strip_prefix('!') {
                    (true, rest)
                } else if let Some(rest) = raw.strip_prefix('\\') {
                    if rest.starts_with('!') || rest.starts_with('\\') {
                        (false, rest)
                    } else {
                        (false, raw.as_str())
                    }
                } else {
                    (false, raw.as_str())
                };

                if pattern.is_empty() {
                    return None;
                }

                Pattern::new(pattern)
                    .ok()
                    .map(|pattern| IgnorePattern { negated, pattern })
            })
            .collect();
        Self { patterns }
    }

    fn matches(&self, value: &str) -> bool {
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: false,
            require_literal_leading_dot: false,
        };

        for entry in self.patterns.iter().rev() {
            if entry.pattern.matches_with(value, options) {
                return !entry.negated;
            }
        }
        false
    }
}

fn is_ignored(
    name: &str,
    groups: &[String],
    ignore_pkg: &IgnorePatterns,
    ignore_group: &IgnorePatterns,
) -> bool {
    ignore_pkg.matches(name) || groups.iter().any(|group| ignore_group.matches(group))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn version(raw: &str) -> Version {
        Version::parse(raw)
    }

    fn installed(name: &str, raw_version: &str) -> InstalledReplacement {
        InstalledReplacement {
            name: name.to_string(),
            version: version(raw_version),
            reason: InstallReason::Explicit,
            groups: Vec::new(),
        }
    }

    fn installed_with(
        name: &str,
        raw_version: &str,
        reason: InstallReason,
        groups: &[&str],
    ) -> InstalledReplacement {
        InstalledReplacement {
            name: name.to_string(),
            version: version(raw_version),
            reason,
            groups: groups.iter().map(|group| group.to_string()).collect(),
        }
    }

    fn candidate(
        name: &str,
        raw_version: &str,
        repo_rank: usize,
        replaces: &[&str],
    ) -> ReplacementCandidate {
        ReplacementCandidate {
            name: name.to_string(),
            version: version(raw_version),
            repo_rank,
            replaces: replaces
                .iter()
                .map(|replace| DepConstraint::parse(replace))
                .collect(),
            provides: Vec::new(),
            conflicts: Vec::new(),
        }
    }

    #[test]
    fn simple_replace_is_selected() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [candidate("new", "2.0-1", 0, &["old"])];

        assert_eq!(
            find_replacements(&installed, &candidates, &[], &[]),
            vec![Replacement {
                old_name: "old".to_string(),
                old_version: version("1.0-1"),
                old_reason: InstallReason::Explicit,
                new_name: "new".to_string(),
                new_version: version("2.0-1"),
                repo_rank: 0,
            }]
        );
    }

    #[test]
    fn versioned_replace_matches_installed_version() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [candidate("new", "2.0-1", 0, &["old<2"])];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].new_name, "new");
    }

    #[test]
    fn versioned_replace_does_not_match_outside_range() {
        let installed = [installed("old", "2.0-1")];
        let candidates = [candidate("new", "2.0-1", 0, &["old<2"])];

        assert!(find_replacements(&installed, &candidates, &[], &[]).is_empty());
    }

    #[test]
    fn unversioned_replace_matches_any_installed_version() {
        let installed = [installed("old", "9.9-9")];
        let candidates = [candidate("new", "1.0-1", 0, &["old"])];

        assert_eq!(
            find_replacements(&installed, &candidates, &[], &[]).len(),
            1
        );
    }

    #[test]
    fn higher_priority_repo_wins() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("lower", "3.0-1", 1, &["old"]),
            candidate("higher", "2.0-1", 0, &["old"]),
        ];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].new_name, "higher");
        assert_eq!(result[0].repo_rank, 0);
    }

    #[test]
    fn lower_priority_replace_is_used_without_a_higher_match() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("unrelated", "1.0-1", 0, &["other"]),
            candidate("lower", "3.0-1", 1, &["old"]),
        ];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].new_name, "lower");
        assert_eq!(result[0].repo_rank, 1);
    }

    #[test]
    fn higher_repo_literal_package_beats_lower_repo_replacement() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("old", "1.5-1", 0, &[]),
            candidate("new", "2.0-1", 1, &["old"]),
        ];

        assert!(find_replacements(&installed, &candidates, &[], &[]).is_empty());
    }

    #[test]
    fn same_repo_replacement_beats_literal_package() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("old", "1.5-1", 0, &[]),
            candidate("new", "2.0-1", 0, &["old"]),
        ];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].new_name, "new");
    }

    #[test]
    fn ignored_installed_package_is_not_replaced() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [candidate("new", "2.0-1", 0, &["old"])];

        assert!(find_replacements(&installed, &candidates, &["old".to_string()], &[]).is_empty());
        assert!(find_replacements(&installed, &candidates, &["o*".to_string()], &[]).is_empty());
    }

    #[test]
    fn ignored_candidate_is_skipped() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("ignored", "2.0-1", 0, &["old"]),
            candidate("accepted", "2.0-1", 0, &["old"]),
        ];

        let result = find_replacements(&installed, &candidates, &["ignored".to_string()], &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].new_name, "accepted");
    }

    #[test]
    fn ignore_group_matches_installed_groups() {
        let installed = [installed_with(
            "old",
            "1.0-1",
            InstallReason::Explicit,
            &["base", "tools"],
        )];
        let candidates = [candidate("new", "2.0-1", 0, &["old"])];

        assert!(find_replacements(&installed, &candidates, &[], &["base".to_string()]).is_empty());
        assert!(find_replacements(&installed, &candidates, &[], &["tool*".to_string()]).is_empty());
        assert_eq!(
            find_replacements(&installed, &candidates, &[], &["other".to_string()]).len(),
            1
        );
    }

    #[test]
    fn all_ignored_replacers_fall_through_to_lower_repo() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("ignored", "2.0-1", 0, &["old"]),
            candidate("accepted", "2.0-1", 1, &["old"]),
        ];

        let result = find_replacements(&installed, &candidates, &["ignored".to_string()], &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].new_name, "accepted");
        assert_eq!(result[0].repo_rank, 1);
    }

    #[test]
    fn multiple_replacers_in_one_repo_are_all_accepted() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("first", "2.0-1", 0, &["old"]),
            candidate("second", "3.0-1", 0, &["old>=1"]),
        ];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        let names: Vec<&str> = result.iter().map(|r| r.new_name.as_str()).collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn one_replacer_can_replace_multiple_installed_packages() {
        let installed = [installed("alpha", "1.0-1"), installed("beta", "1.0-1")];
        let candidates = [candidate("merged", "2.0-1", 0, &["alpha", "beta"])];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        let pairs: Vec<(&str, &str)> = result
            .iter()
            .map(|r| (r.old_name.as_str(), r.new_name.as_str()))
            .collect();
        assert_eq!(pairs, vec![("alpha", "merged"), ("beta", "merged")]);
    }

    #[test]
    fn multiple_installed_packages_are_processed_in_order() {
        let installed = [installed("alpha", "1.0-1"), installed("beta", "1.0-1")];
        let candidates = [
            candidate("new-beta", "2.0-1", 0, &["beta"]),
            candidate("new-alpha", "2.0-1", 0, &["alpha"]),
        ];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        let pairs: Vec<(&str, &str)> = result
            .iter()
            .map(|r| (r.old_name.as_str(), r.new_name.as_str()))
            .collect();
        assert_eq!(pairs, vec![("alpha", "new-alpha"), ("beta", "new-beta")]);
    }

    #[test]
    fn reason_is_copied_from_old_package() {
        let installed = [
            installed_with("explicit-old", "1.0-1", InstallReason::Explicit, &[]),
            installed_with("dep-old", "1.0-1", InstallReason::Dep, &[]),
        ];
        let candidates = [
            candidate("new-explicit", "2.0-1", 0, &["explicit-old"]),
            candidate("new-dep", "2.0-1", 0, &["dep-old"]),
        ];

        let result = find_replacements(&installed, &candidates, &[], &[]);
        assert_eq!(result[0].old_reason, InstallReason::Explicit);
        assert_eq!(result[1].old_reason, InstallReason::Dep);
    }

    #[test]
    fn no_replacement_without_a_matching_replaces_entry() {
        let installed = [installed("old", "1.0-1")];
        let candidates = [
            candidate("old", "1.5-1", 0, &[]),
            candidate("other", "2.0-1", 0, &["unrelated"]),
        ];

        assert!(find_replacements(&installed, &candidates, &[], &[]).is_empty());
    }

    #[test]
    fn plain_install_is_not_handled() {
        let candidates = [candidate("new", "2.0-1", 0, &["old"])];

        assert!(find_replacements(&[], &candidates, &[], &[]).is_empty());
    }

    #[test]
    fn ignore_patterns_use_last_match_negation_and_escape() {
        let patterns = IgnorePatterns::new(&["foo*".to_string(), "!foo-bar".to_string()]);
        assert!(patterns.matches("foo-baz"));
        assert!(!patterns.matches("foo-bar"));
        assert!(!patterns.matches("bar"));

        let patterns = IgnorePatterns::new(&["!foo".to_string(), "foo*".to_string()]);
        assert!(patterns.matches("foo"));

        let patterns = IgnorePatterns::new(&[r"\!literal".to_string()]);
        assert!(patterns.matches("!literal"));
        assert!(!patterns.matches("literal"));

        let patterns = IgnorePatterns::new(&["path/*".to_string()]);
        assert!(patterns.matches("path/to/file"));

        let patterns = IgnorePatterns::new(&["".to_string(), "[".to_string()]);
        assert!(!patterns.matches("anything"));
    }
}
