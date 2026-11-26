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
fn ignore_group_helper_uses_glob_and_negation() {
    let groups = vec!["base".to_string(), "tools".to_string()];

    assert!(matches_ignore_group(&groups, &["base".to_string()]));
    assert!(matches_ignore_group(&groups, &["tool*".to_string()]));
    assert!(matches_ignore_group(
        &groups,
        &["*".to_string(), "!base".to_string()]
    ));
    assert!(!matches_ignore_group(&groups, &["desktop".to_string()]));
    assert!(!matches_ignore_group(&groups, &[]));
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
