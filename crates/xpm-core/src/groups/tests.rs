use super::*;

fn pkg(name: &str, version: &str, repo: Option<&str>, groups: &[&str]) -> GroupPackage {
    GroupPackage {
        name: name.to_string(),
        version: Version::parse(version),
        repo: repo.map(str::to_string),
        groups: groups.iter().map(|group| group.to_string()).collect(),
    }
}

fn names(packages: &[GroupPackage]) -> Vec<String> {
    packages.iter().map(|pkg| pkg.name.clone()).collect()
}

fn versions(packages: &[GroupPackage]) -> Vec<String> {
    packages.iter().map(|pkg| pkg.version.to_string()).collect()
}

#[test]
fn indexes_multiple_groups_per_package() {
    let mut index = GroupIndex::new();
    index.add(pkg("alpha", "1-1", Some("core"), &["base", "tools"]));
    index.add(pkg("beta", "2-1", Some("core"), &["tools"]));

    assert_eq!(index.group_names(), vec!["base", "tools"]);
    assert_eq!(names(&index.expand("base").unwrap()), vec!["alpha"]);
    assert_eq!(
        names(&index.expand("tools").unwrap()),
        vec!["alpha", "beta"]
    );
    assert_eq!(
        index.group_pairs(),
        vec![
            ("base".to_string(), "alpha".to_string()),
            ("tools".to_string(), "alpha".to_string()),
            ("tools".to_string(), "beta".to_string()),
        ]
    );
}

#[test]
fn duplicate_group_entry_is_indexed_once() {
    let mut index = GroupIndex::new();
    index.add(pkg("alpha", "1-1", None, &["base", "base"]));

    assert_eq!(names(&index.expand("base").unwrap()), vec!["alpha"]);
    assert_eq!(index.members("base").len(), 1);
    assert_eq!(index.group_names(), vec!["base"]);
}

#[test]
fn expand_dedupes_by_name_with_first_repo_winning() {
    let index = GroupIndex::from_repo_entries(vec![
        ("core".to_string(), pkg("shared", "1-1", None, &["base"])),
        ("core".to_string(), pkg("core-only", "1-1", None, &["base"])),
        ("extra".to_string(), pkg("shared", "9-1", None, &["base"])),
        (
            "extra".to_string(),
            pkg("extra-only", "1-1", None, &["base"]),
        ),
    ]);

    let expanded = index.expand("base").unwrap();
    assert_eq!(names(&expanded), vec!["shared", "core-only", "extra-only"]);
    assert_eq!(versions(&expanded), vec!["1-1", "1-1", "1-1"]);
    assert_eq!(expanded[0].repo.as_deref(), Some("core"));

    let member_names: Vec<String> = index
        .members("base")
        .iter()
        .map(|pkg| pkg.name.clone())
        .collect();
    assert_eq!(
        member_names,
        vec!["shared", "core-only", "shared", "extra-only"]
    );

    assert_eq!(
        index.group_members_pairs("base"),
        vec![
            ("base".to_string(), "shared".to_string()),
            ("base".to_string(), "core-only".to_string()),
            ("base".to_string(), "shared".to_string()),
            ("base".to_string(), "extra-only".to_string()),
        ]
    );
}

#[test]
fn repo_rank_controls_order_even_for_interleaved_entries() {
    let index = GroupIndex::from_repo_entries(vec![
        (
            "core".to_string(),
            pkg("core-first", "1-1", None, &["base"]),
        ),
        (
            "extra".to_string(),
            pkg("extra-first", "1-1", None, &["base"]),
        ),
        (
            "core".to_string(),
            pkg("core-second", "1-1", None, &["base"]),
        ),
    ]);

    assert_eq!(
        names(&index.expand("base").unwrap()),
        vec!["core-first", "core-second", "extra-first"]
    );
    assert_eq!(
        index.group_members_pairs("base"),
        vec![
            ("base".to_string(), "core-first".to_string()),
            ("base".to_string(), "extra-first".to_string()),
            ("base".to_string(), "core-second".to_string()),
        ]
    );
}

#[test]
fn package_provider_group_precedence() {
    let mut index = GroupIndex::new();
    index.add(pkg("member", "1-1", Some("core"), &["collision"]));
    index.add(pkg("collision", "1-1", Some("core"), &[]));

    let package_names = HashSet::from(["collision".to_string()]);
    let providers = HashSet::from(["collision".to_string()]);

    assert_eq!(
        index.resolve_target("collision", &package_names, &providers),
        TargetResolution::Package
    );
    assert_eq!(
        index.resolve_target("collision", &HashSet::new(), &providers),
        TargetResolution::Provider
    );
    assert_eq!(
        index.resolve_target("collision", &HashSet::new(), &HashSet::new()),
        TargetResolution::Group(vec![pkg("member", "1-1", Some("core"), &["collision"])])
    );
    assert_eq!(
        index.resolve_target("missing", &HashSet::new(), &HashSet::new()),
        TargetResolution::NotFound
    );
}

#[test]
fn missing_and_empty_groups() {
    let index = GroupIndex::from_local(&[pkg("solo", "1-1", None, &[])]);

    assert_eq!(index.group_names(), Vec::<String>::new());
    assert!(index.expand("missing").is_none());
    assert!(index.expand("").is_none());
    assert!(index.members("missing").is_empty());
    assert!(index.group_members_pairs("missing").is_empty());
    assert!(index.group_pairs().is_empty());
}

#[test]
fn local_groups_expand_in_local_order() {
    let local = vec![
        pkg("alpha", "1-1", None, &["desktop"]),
        pkg("beta", "2-1", None, &["desktop", "extra"]),
        pkg("gamma", "3-1", None, &["extra"]),
    ];
    let index = GroupIndex::from_local(&local);

    assert_eq!(
        index.expand("desktop").unwrap(),
        vec![local[0].clone(), local[1].clone()]
    );
    assert_eq!(
        index.expand("extra").unwrap(),
        vec![local[1].clone(), local[2].clone()]
    );
    assert_eq!(
        index.group_pairs(),
        vec![
            ("desktop".to_string(), "alpha".to_string()),
            ("desktop".to_string(), "beta".to_string()),
            ("extra".to_string(), "beta".to_string()),
            ("extra".to_string(), "gamma".to_string()),
        ]
    );
}

#[test]
fn repeated_queries_are_deterministic() {
    let index = GroupIndex::from_repo_entries(vec![
        ("core".to_string(), pkg("one", "1-1", None, &["a", "b"])),
        ("extra".to_string(), pkg("two", "2-1", None, &["b", "c"])),
        ("core".to_string(), pkg("three", "3-1", None, &["c"])),
    ]);

    assert_eq!(index.group_names(), index.group_names());
    assert_eq!(index.group_pairs(), index.group_pairs());
    assert_eq!(index.expand("b"), index.expand("b"));
    assert_eq!(names(&index.expand("c").unwrap()), vec!["three", "two"]);
}
