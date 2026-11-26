use super::*;

fn package(
    name: &str,
    version: &str,
    depends: &[&str],
    provides: &[&str],
    reason: InstallReason,
) -> InstalledPkg {
    InstalledPkg {
        name: name.to_string(),
        version: Version::parse(version),
        depends: depends
            .iter()
            .map(|dependency| DepConstraint::parse(dependency))
            .collect(),
        provides: provides
            .iter()
            .map(|provide| DepConstraint::parse(provide))
            .collect(),
        reason,
    }
}

fn explicit(name: &str, version: &str, depends: &[&str], provides: &[&str]) -> InstalledPkg {
    package(name, version, depends, provides, InstallReason::Explicit)
}

fn as_dep(name: &str, version: &str, depends: &[&str], provides: &[&str]) -> InstalledPkg {
    let mut package = package(name, version, depends, provides, InstallReason::Dep);
    package.reason = InstallReason::Dep;
    package
}

fn targets(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn expect_broken(result: XpmResult<Vec<String>>, message: &str) {
    match result {
        Err(XpmError::DependencyConflict(actual)) => assert_eq!(actual, message),
        other => panic!("expected dependency conflict, got {other:?}"),
    }
}

#[test]
fn provider_is_checked_on_removal() {
    let installed = vec![
        as_dep("virtual-b", "1.0-1", &[], &["virt"]),
        explicit("needs-virt", "1.0-1", &["virt"], &[]),
    ];

    expect_broken(
        plan_removal(
            &installed,
            &targets(&["virtual-b"]),
            &RemoveOptions::default(),
        ),
        "removing virtual-b breaks dependency 'virt' required by needs-virt",
    );
}

#[test]
fn remaining_provider_satisfies_dependency() {
    let installed = vec![
        as_dep("virtual-b", "1.0-1", &[], &["virt"]),
        as_dep("virtual-c", "1.0-1", &[], &["virt"]),
        explicit("needs-virt", "1.0-1", &["virt"], &[]),
    ];

    let plan = plan_removal(
        &installed,
        &targets(&["virtual-b"]),
        &RemoveOptions::default(),
    )
    .expect("removal succeeds");

    assert_eq!(plan, vec!["virtual-b"]);
}

#[test]
fn versioned_provide_must_match_dependency() {
    let installed = vec![
        as_dep("libfoo", "1.0-1", &[], &["foo=3"]),
        explicit("app", "1.0-1", &["foo>=2"], &[]),
    ];

    expect_broken(
        plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default()),
        "removing libfoo breaks dependency 'foo>=2' required by app",
    );
}

#[test]
fn versioned_provide_with_wrong_version_does_not_satisfy() {
    let installed = vec![
        as_dep("libfoo", "1.0-1", &[], &["foo=1"]),
        explicit("app", "1.0-1", &["foo>=2"], &[]),
    ];

    let plan = plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default())
        .expect("dependency was already broken");

    assert_eq!(plan, vec!["libfoo"]);
}

#[test]
fn unversioned_provide_does_not_satisfy_versioned_dependency() {
    let installed = vec![
        as_dep("libfoo", "1.0-1", &[], &["foo"]),
        explicit("app", "1.0-1", &["foo>=2"], &[]),
    ];

    let plan = plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default())
        .expect("dependency was already broken");

    assert_eq!(plan, vec!["libfoo"]);
}

#[test]
fn ignore_versions_still_checks_names() {
    let installed = vec![
        as_dep("libfoo", "1.0-1", &[], &[]),
        explicit("app", "1.0-1", &["libfoo>=2"], &[]),
    ];
    let options = RemoveOptions {
        ignore_versions: true,
        ..RemoveOptions::default()
    };

    expect_broken(
        plan_removal(&installed, &targets(&["libfoo"]), &options),
        "removing libfoo breaks dependency 'libfoo>=2' required by app",
    );
}

#[test]
fn ignore_versions_matches_unversioned_provide() {
    let installed = vec![
        as_dep("libfoo", "1.0-1", &[], &["foo"]),
        explicit("app", "1.0-1", &["foo>=2"], &[]),
    ];
    let options = RemoveOptions {
        ignore_versions: true,
        ..RemoveOptions::default()
    };

    expect_broken(
        plan_removal(&installed, &targets(&["libfoo"]), &options),
        "removing libfoo breaks dependency 'foo>=2' required by app",
    );
}

#[test]
fn skip_deps_ignores_broken_dependencies() {
    let installed = vec![
        as_dep("libfoo", "1.0-1", &[], &[]),
        explicit("app", "1.0-1", &["libfoo>=2"], &[]),
    ];
    let options = RemoveOptions {
        skip_deps: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["libfoo"]), &options).expect("removal succeeds");

    assert_eq!(plan, vec!["libfoo"]);
}

#[test]
fn already_broken_dependency_is_ignored() {
    let installed = vec![
        as_dep("libfoo", "1.0-1", &[], &[]),
        explicit("app", "1.0-1", &["libfoo>=2"], &[]),
    ];

    let plan = plan_removal(&installed, &targets(&["libfoo"]), &RemoveOptions::default())
        .expect("removal succeeds");

    assert_eq!(plan, vec!["libfoo"]);
}

#[test]
fn recursive_removes_chain() {
    let installed = vec![
        explicit("a", "1.0-1", &["b"], &[]),
        as_dep("b", "1.0-1", &["c"], &[]),
        as_dep("c", "1.0-1", &[], &[]),
    ];
    let options = RemoveOptions {
        recursive: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");

    assert_eq!(plan, vec!["a", "b", "c"]);
}

#[test]
fn recursive_keeps_explicit_dependency() {
    let installed = vec![
        explicit("a", "1.0-1", &["b"], &[]),
        as_dep("b", "1.0-1", &["c"], &[]),
        explicit("c", "1.0-1", &[], &[]),
    ];
    let options = RemoveOptions {
        recursive: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");
    assert_eq!(plan, vec!["a", "b"]);

    let options = RemoveOptions {
        recursive: true,
        recursive_all: true,
        ..RemoveOptions::default()
    };
    let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");
    assert_eq!(plan, vec!["a", "b", "c"]);
}

#[test]
fn recursive_keeps_shared_dependency_and_its_dependencies() {
    let installed = vec![
        explicit("a", "1.0-1", &["b"], &[]),
        as_dep("b", "1.0-1", &["c"], &[]),
        as_dep("c", "1.0-1", &[], &[]),
        explicit("e", "1.0-1", &["b"], &[]),
    ];
    let options = RemoveOptions {
        recursive: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");

    assert_eq!(plan, vec!["a"]);
}

#[test]
fn recursive_cycle_terminates() {
    let installed = vec![
        as_dep("a", "1.0-1", &["b"], &[]),
        as_dep("b", "1.0-1", &["a"], &[]),
    ];
    let options = RemoveOptions {
        recursive: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["a"]), &options).expect("removal succeeds");

    assert_eq!(plan, vec!["a", "b"]);
}

#[test]
fn cascade_removes_dependents() {
    let installed = vec![
        explicit("a", "1.0-1", &["b"], &[]),
        as_dep("b", "1.0-1", &[], &[]),
    ];
    let options = RemoveOptions {
        cascade: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["b"]), &options).expect("removal succeeds");

    assert_eq!(plan, vec!["a", "b"]);
}

#[test]
fn cascade_then_recursive_removes_chain() {
    let installed = vec![
        explicit("a", "1.0-1", &["b"], &[]),
        as_dep("b", "1.0-1", &["c"], &[]),
        as_dep("c", "1.0-1", &[], &[]),
    ];
    let options = RemoveOptions {
        cascade: true,
        recursive: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["b"]), &options).expect("removal succeeds");

    assert_eq!(plan, vec!["a", "b", "c"]);
}

#[test]
fn unneeded_keeps_required_target() {
    let installed = vec![
        as_dep("a", "1.0-1", &[], &[]),
        as_dep("b", "1.0-1", &[], &[]),
        explicit("c", "1.0-1", &["a"], &[]),
    ];
    let options = RemoveOptions {
        unneeded: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["a", "b"]), &options).expect("removal succeeds");

    assert_eq!(plan, vec!["b"]);
}

#[test]
fn unneeded_with_all_targets_required_is_empty() {
    let installed = vec![
        as_dep("a", "1.0-1", &[], &[]),
        as_dep("b", "1.0-1", &[], &[]),
        explicit("c", "1.0-1", &["a"], &[]),
        explicit("d", "1.0-1", &["b"], &[]),
    ];
    let options = RemoveOptions {
        unneeded: true,
        ..RemoveOptions::default()
    };

    let plan = plan_removal(&installed, &targets(&["a", "b"]), &options).expect("removal succeeds");

    assert!(plan.is_empty());
}

#[test]
fn target_and_its_dependency_can_be_removed_together() {
    let installed = vec![
        explicit("app", "1.0-1", &["libfoo>=2"], &[]),
        as_dep("libfoo", "2.0-1", &[], &[]),
    ];

    let plan = plan_removal(
        &installed,
        &targets(&["app", "libfoo"]),
        &RemoveOptions::default(),
    )
    .expect("removal succeeds");

    assert_eq!(plan, vec!["app", "libfoo"]);
}

#[test]
fn unknown_target_is_an_error() {
    let installed = vec![explicit("a", "1.0-1", &[], &[])];

    let error = plan_removal(
        &installed,
        &targets(&["missing"]),
        &RemoveOptions::default(),
    )
    .expect_err("unknown target");

    match error {
        XpmError::Package(message) => assert_eq!(message, "target not found: missing"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn duplicate_targets_are_removed_once() {
    let installed = vec![explicit("a", "1.0-1", &[], &[])];

    let plan = plan_removal(
        &installed,
        &targets(&["a", "a", "a"]),
        &RemoveOptions::default(),
    )
    .expect("removal succeeds");

    assert_eq!(plan, vec!["a"]);
}

#[test]
fn multiple_broken_dependencies_are_joined_with_newlines() {
    let installed = vec![
        as_dep("a", "1.0-1", &[], &[]),
        as_dep("b", "1.0-1", &[], &[]),
        explicit("c", "1.0-1", &["a"], &[]),
        explicit("d", "1.0-1", &["b"], &[]),
    ];

    expect_broken(
            plan_removal(
                &installed,
                &targets(&["a", "b"]),
                &RemoveOptions::default(),
            ),
            "removing a breaks dependency 'a' required by c\nremoving b breaks dependency 'b' required by d",
        );
}

#[test]
fn self_provided_dependency_is_not_broken() {
    let installed = vec![
        as_dep("a", "1.0-1", &[], &[]),
        explicit("c", "1.0-1", &["virt"], &["virt=1"]),
    ];

    let plan = plan_removal(&installed, &targets(&["a"]), &RemoveOptions::default())
        .expect("removal succeeds");

    assert_eq!(plan, vec!["a"]);
}

#[test]
fn sorted_dependents_before_dependencies() {
    let installed = vec![
        as_dep("z-dependency", "1.0-1", &[], &[]),
        explicit("a-dependent", "1.0-1", &["z-dependency"], &[]),
        explicit("m-dependent", "1.0-1", &["z-dependency"], &[]),
    ];

    let plan = plan_removal(
        &installed,
        &targets(&["a-dependent", "m-dependent", "z-dependency"]),
        &RemoveOptions::default(),
    )
    .expect("removal succeeds");

    assert_eq!(plan, vec!["m-dependent", "a-dependent", "z-dependency"]);
}
