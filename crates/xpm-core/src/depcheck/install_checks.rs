use super::*;

fn installed(name: &str, version: &str, provides: &[&str]) -> InstalledPkg {
    InstalledPkg {
        name: name.to_string(),
        version: Version::parse(version),
        depends: vec![],
        provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
        reason: InstallReason::Explicit,
    }
}

fn target(
    name: &str,
    version: &str,
    depends: &[&str],
    provides: &[&str],
    conflicts: &[&str],
) -> InstallTarget {
    InstallTarget {
        name: name.to_string(),
        version: Version::parse(version),
        depends: depends.iter().map(|d| DepConstraint::parse(d)).collect(),
        provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
        conflicts: conflicts.iter().map(|c| DepConstraint::parse(c)).collect(),
    }
}

#[test]
fn local_target_missing_dependency_is_reported() {
    let err = check_install(&[], &[target("a", "1-1", &["b"], &[], &[])]).unwrap_err();
    assert!(matches!(err, XpmError::DependencyConflict(_)));
}

#[test]
fn installed_package_satisfies_local_dependency() {
    check_install(
        &[installed("b", "1-1", &["virt"])],
        &[target("a", "1-1", &["b"], &[], &[])],
    )
    .expect("installed b satisfies a");
}

#[test]
fn target_satisfies_another_target_dependency() {
    check_install(
        &[],
        &[
            target("a", "1-1", &["b"], &[], &[]),
            target("b", "1-1", &[], &["virt"], &[]),
        ],
    )
    .expect("target b satisfies a");
}

#[test]
fn versioned_dependency_requires_versioned_provide() {
    let err = check_install(
        &[installed("b", "1-1", &["virt"])],
        &[target("a", "1-1", &["virt>=2"], &[], &[])],
    )
    .unwrap_err();
    assert!(matches!(err, XpmError::DependencyConflict(_)));
}

#[test]
fn target_conflict_with_installed_is_reported() {
    let err = check_install(
        &[installed("b", "1-1", &[])],
        &[target("a", "1-1", &[], &[], &["b"])],
    )
    .unwrap_err();
    assert!(matches!(err, XpmError::DependencyConflict(_)));
}

#[test]
fn target_conflict_with_other_target_is_reported() {
    let err = check_install(
        &[],
        &[
            target("a", "1-1", &[], &[], &["b"]),
            target("b", "1-1", &[], &[], &[]),
        ],
    )
    .unwrap_err();
    assert!(matches!(err, XpmError::DependencyConflict(_)));
}

#[test]
fn upgrade_that_breaks_reverse_dependency_is_reported() {
    let mut app = installed("app", "1-1", &[]);
    app.depends = vec![DepConstraint::parse("lib=1")];
    let mut lib = installed("lib", "1-1", &[]);
    lib.provides = vec![];
    let new_lib = InstalledPkg {
        name: "lib".to_string(),
        version: Version::parse("2-1"),
        depends: vec![],
        provides: vec![],
        reason: InstallReason::Explicit,
    };

    let err = check_installed_closure(&[app, lib], &["lib".to_string()], &[new_lib])
        .expect_err("lib=1 is no longer satisfied");
    assert!(matches!(err, XpmError::DependencyConflict(_)));
    assert!(format!("{err}").contains("removing lib breaks dependency 'lib=1' required by app"));
}

#[test]
fn replacement_providing_old_name_satisfies_reverse_dependency() {
    let mut app = installed("app", "1-1", &[]);
    app.depends = vec![DepConstraint::parse("old")];
    let old = installed("old", "1-1", &[]);
    let new = InstalledPkg {
        name: "new".to_string(),
        version: Version::parse("1-1"),
        depends: vec![],
        provides: vec![DepConstraint::parse("old")],
        reason: InstallReason::Explicit,
    };

    check_installed_closure(&[app, old], &["old".to_string()], &[new]).expect("new provides old");
}

#[test]
fn self_provide_conflict_is_ignored() {
    check_install(&[], &[target("udev", "1-1", &[], &["udev"], &["udev"])])
        .expect("a package does not conflict with itself");
}
