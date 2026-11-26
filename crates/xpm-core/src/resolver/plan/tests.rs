use super::*;

fn candidate(name: &str, version: &str, deps: &[&str], provides: &[&str]) -> PackageCandidate {
    PackageCandidate {
        name: name.to_string(),
        version: Version::parse(version),
        repo: None,
        depends: deps.iter().map(|d| DepConstraint::parse(d)).collect(),
        conflicts: vec![],
        provides: provides.iter().map(|p| DepConstraint::parse(p)).collect(),
        replaces: vec![],
        optdepends: vec![],
    }
}

fn plan_names(plan: &[(PackageCandidate, bool)]) -> Vec<String> {
    plan.iter()
        .map(|(candidate, _)| format!("{}={}", candidate.name, candidate.version))
        .collect()
}

#[test]
fn resolves_transitive_dependencies_in_order() {
    let candidates = vec![
        candidate("app", "1.0-1", &["lib", "runtime"], &[]),
        candidate("lib", "2.0-1", &["runtime"], &[]),
        candidate("runtime", "3.0-1", &[], &[]),
    ];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    let names = plan_names(&plan);

    let runtime = names
        .iter()
        .position(|n| n.starts_with("runtime="))
        .unwrap();
    let lib = names.iter().position(|n| n.starts_with("lib=")).unwrap();
    let app = names.iter().position(|n| n.starts_with("app=")).unwrap();
    assert!(runtime < lib, "runtime must precede lib: {names:?}");
    assert!(lib < app, "lib must precede app: {names:?}");
}

#[test]
fn marks_requested_packages_explicit_and_dependencies_not() {
    let candidates = vec![
        candidate("app", "1.0-1", &["lib"], &[]),
        candidate("lib", "1.0-1", &[], &[]),
    ];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    let app = plan.iter().find(|(c, _)| c.name == "app").unwrap();
    let lib = plan.iter().find(|(c, _)| c.name == "lib").unwrap();
    assert!(app.1, "requested package is explicit");
    assert!(!lib.1, "pulled dependency is not explicit");
}

#[test]
fn exact_version_requirement_selects_that_candidate() {
    let candidates = vec![
        candidate("tool", "1.0-1", &[], &[]),
        candidate("tool", "2.0-1", &[], &[]),
    ];
    let requests = vec![Requirement::parse("tool=1.0-1").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.version, Version::parse("1.0-1"));
}

#[test]
fn unversioned_provides_satisfy_dependencies() {
    let candidates = vec![
        candidate("app", "1.0-1", &["sh"], &[]),
        candidate("bash", "5.2-1", &[], &["sh"]),
    ];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    let names = plan_names(&plan);
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names[0].starts_with("bash="), "{names:?}");
}

#[test]
fn requesting_a_virtual_name_marks_the_provider_explicit() {
    let candidates = vec![candidate("openssh", "9.0-1", &[], &["ssh"])];
    let requests = vec![Requirement::parse("ssh").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "openssh");
    assert!(plan[0].1);
}

#[test]
fn unknown_package_is_reported() {
    let candidates = vec![candidate("app", "1.0-1", &[], &[])];
    let requests = vec![Requirement::parse("missing").unwrap()];

    let error = resolve_closure(candidates, &requests).unwrap_err();
    assert!(matches!(error, XpmError::PackageNotFound { name } if name == "missing"));
}

#[test]
fn unsatisfiable_dependency_is_reported() {
    let candidates = vec![candidate("app", "1.0-1", &["ghost"], &[])];
    let requests = vec![Requirement::parse("app").unwrap()];

    let error = resolve_closure(candidates, &requests).unwrap_err();
    assert!(matches!(error, XpmError::DependencyConflict(_)));
}

#[test]
fn versioned_provides_satisfy_versioned_dependencies() {
    // Real Arch pattern: bash depends on the readline soname, and the
    // readline package provides it with its own version, unrelated to
    // the package version.
    let candidates = vec![
        candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
        candidate("libfoo", "1.2.3-1", &[], &["libfoo.so=8-64"]),
    ];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    let names = plan_names(&plan);
    assert_eq!(names, vec!["libfoo=1.2.3-1", "app=1.0-1"]);
}

#[test]
fn versioned_provides_reject_mismatched_versions() {
    let candidates = vec![
        candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
        candidate("libfoo", "1.2.3-1", &[], &["libfoo.so=7-64"]),
    ];
    let requests = vec![Requirement::parse("app").unwrap()];

    let error = resolve_closure(candidates, &requests).unwrap_err();
    assert!(matches!(error, XpmError::DependencyConflict(_)));
}

#[test]
fn unversioned_provide_does_not_satisfy_versioned_dependency() {
    let candidates = vec![
        candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
        candidate("libfoo", "1.2.3-1", &[], &["libfoo.so"]),
    ];
    let requests = vec![Requirement::parse("app").unwrap()];

    let error = resolve_closure(candidates, &requests).unwrap_err();
    assert!(matches!(error, XpmError::DependencyConflict(_)));
}

#[test]
fn unversioned_dependency_accepts_versioned_provide() {
    let candidates = vec![
        candidate("app", "1.0-1", &["sh"], &[]),
        candidate("bash", "5.3-1", &[], &["sh=5.3"]),
    ];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    let names = plan_names(&plan);
    assert_eq!(names, vec!["bash=5.3-1", "app=1.0-1"]);
}

#[test]
fn conflicts_match_versioned_provides() {
    let mut conflicting = candidate("other", "1.0-1", &["libfoo.so=8-64"], &[]);
    conflicting.conflicts = vec![DepConstraint::parse("libfoo.so=8-64")];

    let candidates = vec![
        candidate("app", "1.0-1", &["libfoo.so=8-64"], &[]),
        candidate("libfoo", "1.2.3-1", &[], &["libfoo.so=8-64"]),
        conflicting,
    ];
    let requests = vec![
        Requirement::parse("app").unwrap(),
        Requirement::parse("other").unwrap(),
    ];

    let error = resolve_closure(candidates, &requests).unwrap_err();
    assert!(matches!(error, XpmError::DependencyConflict(_)));
}

#[test]
fn installed_dependency_is_kept_even_when_repo_has_newer() {
    let candidates = vec![
        candidate("app", "1.0-1", &["lib"], &[]),
        candidate("lib", "3.0-1", &[], &[]),
    ];
    let installed = vec![candidate("lib", "2.0-1", &[], &[])];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
    let install: Vec<_> = plan
        .iter()
        .filter(|(_, _, installed)| !*installed)
        .map(|(c, _, _)| c.name.clone())
        .collect();
    assert_eq!(install, vec!["app"]);
    assert!(plan
        .iter()
        .any(|(c, _, installed)| c.name == "lib" && *installed));
}

#[test]
fn installed_version_is_replaced_when_constraint_requires_newer() {
    let candidates = vec![
        candidate("app", "1.0-1", &["lib>=3"], &[]),
        candidate("lib", "3.0-1", &[], &[]),
    ];
    let installed = vec![candidate("lib", "2.0-1", &[], &[])];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
    let lib = plan.iter().find(|(c, _, _)| c.name == "lib").unwrap();
    assert_eq!(lib.0.version, Version::parse("3.0-1"));
    assert!(!lib.2, "the newer repository candidate must be selected");
}

#[test]
fn explicit_request_prefers_newer_repository_candidate() {
    let candidates = vec![candidate("foo", "2.0-1", &[], &[])];
    let installed = vec![candidate("foo", "1.0-1", &[], &[])];
    let requests = vec![Requirement::parse("foo").unwrap()];

    let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.version, Version::parse("2.0-1"));
    assert!(!plan[0].2);
}

#[test]
fn explicit_request_does_not_downgrade_below_installed() {
    let candidates = vec![candidate("foo", "2.0-1", &[], &[])];
    let installed = vec![candidate("foo", "3.0-1", &[], &[])];
    let requests = vec![Requirement::parse("foo").unwrap()];

    let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.version, Version::parse("3.0-1"));
    assert!(plan[0].2);
}

#[test]
fn installed_provide_satisfies_dependency() {
    let candidates = vec![
        candidate("app", "1.0-1", &["sh"], &[]),
        candidate("bash", "6.0-1", &[], &["sh"]),
    ];
    let installed = vec![candidate("bash", "5.3-1", &[], &["sh"])];
    let requests = vec![Requirement::parse("app").unwrap()];

    let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
    let bash = plan.iter().find(|(c, _, _)| c.name == "bash").unwrap();
    assert_eq!(bash.0.version, Version::parse("5.3-1"));
    assert!(bash.2);
}

#[test]
fn requirement_parser_rejects_empty_specs() {
    assert!(matches!(Requirement::parse(""), Err(XpmError::Package(_))));
    assert!(matches!(
        Requirement::parse("=1.0"),
        Err(XpmError::Package(_))
    ));
    assert!(matches!(
        Requirement::parse("foo="),
        Err(XpmError::Package(_))
    ));
    assert!(Requirement::parse("foo=1.0").is_ok());
}

#[test]
fn requirement_parser_accepts_repo_qualified_specs() {
    let req = Requirement::parse("extra/power-profiles-daemon").unwrap();
    assert_eq!(req.repo.as_deref(), Some("extra"));
    assert_eq!(req.name, "power-profiles-daemon");
    assert!(req.version.is_none());

    let pinned = Requirement::parse("extra/power-profiles-daemon=0.30-1").unwrap();
    assert_eq!(pinned.repo.as_deref(), Some("extra"));
    assert_eq!(pinned.name, "power-profiles-daemon");
    assert_eq!(pinned.version, Some(Version::parse("0.30-1")));

    assert!(matches!(
        Requirement::parse("extra/"),
        Err(XpmError::Package(_))
    ));
    assert!(matches!(
        Requirement::parse("/foo"),
        Err(XpmError::Package(_))
    ));
}

#[test]
fn exact_name_beats_provider() {
    // power-profiles-daemon (real) vs tuned-ppd (provides it).
    let candidates = vec![
        candidate("power-profiles-daemon", "0.30-1", &[], &[]),
        candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]),
        candidate("tuned", "2.28.0-1", &[], &[]),
    ];
    let requests = vec![Requirement::parse("power-profiles-daemon").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    let names = plan_names(&plan);
    assert_eq!(names, vec!["power-profiles-daemon=0.30-1"]);
}

#[test]
fn pinned_exact_name_beats_versioned_provider() {
    let candidates = vec![
        candidate("power-profiles-daemon", "0.30-1", &[], &[]),
        candidate(
            "tuned-ppd",
            "2.28.0-1",
            &[],
            &["power-profiles-daemon=0.30-1"],
        ),
    ];
    let requests = vec![Requirement::parse("power-profiles-daemon=0.30-1").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "power-profiles-daemon");
}

#[test]
fn exact_name_beats_installed_provider() {
    let candidates = vec![candidate("power-profiles-daemon", "0.30-1", &[], &[])];
    let installed = vec![candidate(
        "tuned-ppd",
        "2.28.0-1",
        &[],
        &["power-profiles-daemon"],
    )];
    let requests = vec![Requirement::parse("power-profiles-daemon").unwrap()];

    let plan = resolve_closure_with_installed(candidates, installed, &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "power-profiles-daemon");
    assert!(!plan[0].2, "the repository package must be installed");
}

#[test]
fn provider_is_used_when_no_exact_name_exists() {
    let candidates = vec![
        candidate("provider-a", "1.0-1", &[], &["virtual"]),
        candidate("provider-b", "2.0-1", &[], &["virtual"]),
    ];
    let requests = vec![Requirement::parse("virtual").unwrap()];

    let plan = resolve_closure(candidates, &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "provider-b");
    assert!(plan[0].1, "the provider is the explicit target");
}

#[test]
fn repo_qualified_request_selects_the_named_repo() {
    let mut core = candidate("foo", "1.0-1", &[], &[]);
    core.repo = Some("core".to_string());
    let mut extra = candidate("foo", "1.0-1", &[], &[]);
    extra.repo = Some("extra".to_string());

    let requests = vec![Requirement::parse("extra/foo").unwrap()];
    let plan = resolve_closure(vec![core, extra], &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.repo.as_deref(), Some("extra"));
}

#[test]
fn repo_qualified_request_does_not_accept_providers() {
    let mut provider = candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]);
    provider.repo = Some("extra".to_string());

    let requests = vec![Requirement::parse("extra/power-profiles-daemon").unwrap()];
    let error = resolve_closure(vec![provider], &requests).unwrap_err();
    assert!(matches!(error, XpmError::PackageNotFound { .. }));
}

#[test]
fn provider_conflicting_with_installed_is_not_selected() {
    let mut conflicting = candidate("tuned-ppd", "2.28.0-1", &[], &["ppd-api"]);
    conflicting.conflicts = vec![DepConstraint::parse("power-profiles-daemon")];
    let alternative = candidate("other-ppd", "1.0-1", &[], &["ppd-api"]);
    let installed = vec![candidate("power-profiles-daemon", "0.30-1", &[], &[])];

    let requests = vec![Requirement::parse("ppd-api").unwrap()];
    let plan = resolve_closure_with_installed(vec![conflicting, alternative], installed, &requests)
        .unwrap();

    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "other-ppd");
    assert!(!plan[0].2);
}

#[test]
fn blocked_requirement_reports_the_installed_conflict() {
    let mut conflicting = candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]);
    conflicting.conflicts = vec![DepConstraint::parse("power-profiles-daemon")];
    let installed = vec![candidate("power-profiles-daemon", "0.30-1", &[], &[])];

    let requests = vec![Requirement::parse("tuned-ppd").unwrap()];
    let error =
        resolve_closure_with_installed(vec![conflicting], installed, &requests).unwrap_err();
    match error {
        XpmError::DependencyConflict(message) => {
            assert!(message.contains("power-profiles-daemon"), "{message}");
            assert!(message.contains("conflicts"), "{message}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn parses_alpm_version_operators_in_requirements() {
    let ge = Requirement::parse("foo>=1.2-1").unwrap();
    assert_eq!(ge.name, "foo");
    assert_eq!(ge.op, Some(Operator::Ge));
    assert_eq!(ge.version.as_ref().unwrap().to_string(), "1.2-1");

    let gt = Requirement::parse("extra/foo>2.0").unwrap();
    assert_eq!(gt.repo.as_deref(), Some("extra"));
    assert_eq!(gt.op, Some(Operator::Gt));
    assert!(Requirement::parse("foo=").is_err());
}

#[test]
fn operator_requirement_matches_a_newer_installed_candidate() {
    let pacman = candidate("pacman", "7.1.0-1", &[], &[]);
    let requests = vec![Requirement::parse("pacman>6.1").unwrap()];
    let plan = resolve_closure(vec![pacman], &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "pacman");
}

#[test]
fn provider_tie_break_prefers_repository_priority_then_name() {
    let mut core = candidate("zzz-core-provider", "1.0-1", &[], &["virtual"]);
    core.repo = Some("core".to_string());
    let mut extra = candidate("aaa-extra-provider", "1.0-1", &[], &["virtual"]);
    extra.repo = Some("extra".to_string());

    // `core` is inserted first, so it has repository priority over `extra`
    // even though the other provider sorts first by name.
    let requests = vec![Requirement::parse("virtual").unwrap()];
    let plan = resolve_closure(vec![core, extra], &requests).unwrap();
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].0.name, "zzz-core-provider");
    assert_eq!(plan[0].0.repo.as_deref(), Some("core"));
}

#[test]
fn self_providing_conflict_between_targets_is_reported() {
    // tuned-ppd provides and conflicts power-profiles-daemon; the resolver
    // skips the self-conflict, so the post-solve check must catch the pair.
    let mut tuned_ppd = candidate("tuned-ppd", "2.28.0-1", &[], &["power-profiles-daemon"]);
    tuned_ppd.conflicts = vec![DepConstraint::parse("power-profiles-daemon")];
    let ppd = candidate("power-profiles-daemon", "0.30-1", &[], &[]);

    let requests = vec![
        Requirement::parse("tuned-ppd").unwrap(),
        Requirement::parse("power-profiles-daemon").unwrap(),
    ];
    let error = resolve_closure(vec![tuned_ppd, ppd], &requests).unwrap_err();
    assert!(matches!(error, XpmError::DependencyConflict(_)));
}
