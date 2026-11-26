use super::*;
use tempfile::TempDir;

fn package(name: &str, version: &str, reason: InstallReason) -> LocalPackage {
    LocalPackage {
        name: name.to_string(),
        version: version.to_string(),
        reason,
        depends: Vec::new(),
        provides: Vec::new(),
        files: Vec::new(),
    }
}

fn with_depends(mut package: LocalPackage, depends: &[&str]) -> LocalPackage {
    package.depends = depends.iter().map(|d| d.to_string()).collect();
    package
}

#[test]
fn normalizes_absolute_relative_and_root_paths() {
    let root = Path::new("/mnt/x");

    assert_eq!(
        normalize_query_path("/usr/bin/tool", root).as_deref(),
        Some("usr/bin/tool")
    );
    assert_eq!(
        normalize_query_path("usr/bin/tool/", root).as_deref(),
        Some("usr/bin/tool")
    );
    assert_eq!(
        normalize_query_path("/mnt/x/usr/bin/tool", root).as_deref(),
        Some("usr/bin/tool")
    );
    assert_eq!(
        normalize_query_path("./usr/./bin/../lib/tool", root).as_deref(),
        Some("usr/lib/tool")
    );
    assert_eq!(normalize_query_path("../escape", root), None);
    assert_eq!(normalize_query_path("/", root), None);
}

#[test]
fn owner_lookup_matches_files_directories_and_basenames() {
    let mut shell = package("bash", "5.2-1", InstallReason::Explicit);
    shell.files = vec![
        "usr/".to_string(),
        "usr/bin/".to_string(),
        "usr/bin/bash".to_string(),
    ];
    let mut util = package("coreutils", "9.5-1", InstallReason::Explicit);
    util.files = vec!["usr/bin/ls".to_string()];
    let packages = vec![shell, util];
    let root = Path::new("/");

    let owners = find_owners(&packages, root, "/usr/bin/bash");
    assert_eq!(
        owners,
        vec![Owner {
            name: "bash".to_string(),
            version: "5.2-1".to_string()
        }]
    );

    let owners = find_owners(&packages, root, "usr/bin/");
    assert_eq!(
        owners
            .iter()
            .map(|owner| owner.name.as_str())
            .collect::<Vec<_>>(),
        vec!["bash"]
    );

    let owners = find_owners(&packages, root, "ls");
    assert_eq!(
        owners
            .iter()
            .map(|owner| owner.name.as_str())
            .collect::<Vec<_>>(),
        vec!["coreutils"]
    );

    assert!(find_owners(&packages, root, "/usr/bin/nope").is_empty());
}

#[test]
fn owner_lookup_uses_the_configured_root() {
    let mut package = package("hello", "1.0-1", InstallReason::Explicit);
    package.files = vec!["usr/bin/hello".to_string()];
    let packages = vec![package];

    let owners = find_owners(&packages, Path::new("/mnt/root"), "/mnt/root/usr/bin/hello");
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0].name, "hello");
}

#[test]
fn foreign_detection_is_by_name_only() {
    let packages = vec![
        package("core-pkg", "9.9-1", InstallReason::Explicit),
        package("local-only", "1.0-1", InstallReason::Explicit),
    ];
    let sync_names = HashSet::from(["core-pkg".to_string()]);

    assert_eq!(
        find_foreign(&packages, &sync_names),
        vec![("local-only".to_string(), "1.0-1".to_string())]
    );
}

#[test]
fn unrequired_uses_only_direct_other_package_dependencies() {
    let packages = vec![
        with_depends(package("app", "1.0-1", InstallReason::Explicit), &["liba"]),
        with_depends(package("liba", "1.0-1", InstallReason::Dep), &["libb"]),
        package("libb", "1.0-1", InstallReason::Dep),
        package("explicit-extra", "1.0-1", InstallReason::Explicit),
    ];

    // liba is required by app; libb is required by liba even though liba
    // itself is only a dependency. app and explicit-extra have no reverse
    // dependencies, while liba/libb are filtered out by --deps.
    assert_eq!(
        find_unrequired(&packages, false),
        vec!["app".to_string(), "explicit-extra".to_string()]
    );
    assert!(find_unrequired(&packages, true).is_empty());
}

#[test]
fn unrequired_skips_transitive_orphans_that_only_depend_on_each_other() {
    let packages = vec![
        package("app", "1.0-1", InstallReason::Explicit),
        with_depends(
            package("orphan-a", "1.0-1", InstallReason::Dep),
            &["orphan-b"],
        ),
        package("orphan-b", "1.0-1", InstallReason::Dep),
    ];

    // orphan-a is not required by anything, so it is unrequired; orphan-b
    // is still directly required by orphan-a. This differs from --orphans,
    // which would report both.
    assert_eq!(
        find_unrequired(&packages, true),
        vec!["orphan-a".to_string()]
    );
}

#[test]
fn unrequired_honours_provides_and_deps_only_filter() {
    let mut browser = package("firefox", "128.0-1", InstallReason::Dep);
    browser.provides = vec!["browser=128.0".to_string()];
    let packages = vec![
        with_depends(
            package("app", "1.0-1", InstallReason::Explicit),
            &["browser>=120"],
        ),
        browser,
        package("dep-extra", "1.0-1", InstallReason::Dep),
        package("explicit-extra", "1.0-1", InstallReason::Explicit),
    ];

    assert_eq!(
        find_unrequired(&packages, true),
        vec!["dep-extra".to_string()]
    );
    assert_eq!(
        find_unrequired(&packages, false),
        vec![
            "app".to_string(),
            "dep-extra".to_string(),
            "explicit-extra".to_string()
        ]
    );
}

#[test]
fn self_dependency_does_not_keep_a_package_required() {
    let packages = vec![with_depends(
        package("selfish", "1.0-1", InstallReason::Dep),
        &["selfish"],
    )];

    assert_eq!(
        find_unrequired(&packages, true),
        vec!["selfish".to_string()]
    );
}

#[test]
fn load_local_packages_reads_metadata_and_files() {
    let tmp = TempDir::new().expect("tmp");
    let local_db = tmp.path().join("local");
    let pkg_dir = local_db.join("hello");
    fs::create_dir_all(&pkg_dir).expect("pkg dir");
    fs::write(pkg_dir.join("version"), "1.0-1\n").expect("version");
    InstallReason::Dep
        .write(&local_db, "hello")
        .expect("reason");
    local_db::write_depends(&local_db, "hello", &["glibc>=2.39".to_string()]).expect("depends");
    local_db::write_provides(&local_db, "hello", &["hello-bin".to_string()]).expect("provides");
    local_db::write_file_list(&local_db, "hello", &["usr/bin/hello".to_string()]).expect("files");

    let packages = load_local_packages(&local_db).expect("load");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0].name, "hello");
    assert_eq!(packages[0].version, "1.0-1");
    assert_eq!(packages[0].reason, InstallReason::Dep);
    assert_eq!(packages[0].depends, vec!["glibc>=2.39".to_string()]);
    assert_eq!(packages[0].provides, vec!["hello-bin".to_string()]);
    assert_eq!(packages[0].files, vec!["usr/bin/hello".to_string()]);
}
