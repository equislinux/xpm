use super::*;

#[test]
fn sync_listing_groups_native_file_changelog_and_package_queries() {
    let fixture = Fixture::new();
    fixture.add_installed("hello", "1.0-1");
    fixture.add_installed("local-only", "3.0-1");
    fixture.write_sync_db(
        "core",
        &[Entry::new("hello", "1.0-1").with_groups(&["base"])],
    );
    fixture.write_sync_db(
        "extra",
        &[Entry::new("tool", "2.0-1").with_groups(&["desktop"])],
    );

    let list = fixture.run(&["-Sl"]);
    assert_success(&list);
    let list_stdout = stdout(&list);
    assert!(list_stdout.contains("core hello 1.0-1 [installed]"));
    assert!(list_stdout.contains("extra tool 2.0-1"));
    assert!(!list_stdout.contains("extra tool 2.0-1 [installed]"));

    let list_quiet = fixture.run(&["-Slq", "core"]);
    assert_success(&list_quiet);
    assert_eq!(stdout(&list_quiet), "hello\n");

    let missing_repo = fixture.run(&["-Sl", "missing"]);
    assert_eq!(missing_repo.status.code(), Some(1));
    assert!(stderr(&missing_repo).contains("repository 'missing' was not found"));

    let groups = fixture.run(&["-Sg"]);
    assert_success(&groups);
    assert_eq!(stdout(&groups), "base\ndesktop\n");

    let groups_all = fixture.run(&["-Sgg"]);
    assert_success(&groups_all);
    assert_eq!(stdout(&groups_all), "base hello\ndesktop tool\n");

    let group_members = fixture.run(&["-Sgq", "desktop"]);
    assert_success(&group_members);
    assert_eq!(stdout(&group_members), "tool\n");

    let native = fixture.run(&["-Qn"]);
    assert_success(&native);
    assert_eq!(stdout(&native), "hello 1.0-1\n");

    let native_quiet = fixture.run(&["-Qnq"]);
    assert_success(&native_quiet);
    assert_eq!(stdout(&native_quiet), "hello\n");

    let changed = fixture.local_db().join("hello").join("changelog");
    fs::write(&changed, "1.0-1: initial release\n").expect("changelog");
    let changelog = fixture.run(&["-Qc", "hello"]);
    assert_success(&changelog);
    assert_eq!(stdout(&changelog), "1.0-1: initial release\n");

    let no_changelog = fixture.run(&["-Qc", "local-only"]);
    assert_eq!(no_changelog.status.code(), Some(1));
    assert!(stderr(&no_changelog).contains("no changelog available for 'local-only'"));

    let entry = Entry::new("hello", "1.0-1");
    let package = fixture._tmp.path().join("hello.pkg.tar.zst");
    fs::write(&package, build_package(&fixture.arch, &entry)).expect("package file");
    let package_query = fixture.run(&["-Qp", package.to_str().unwrap()]);
    assert_success(&package_query);
    assert_eq!(stdout(&package_query), "hello 1.0-1\n");

    let package_quiet = fixture.run(&["-Qpq", package.to_str().unwrap()]);
    assert_success(&package_quiet);
    assert_eq!(stdout(&package_quiet), "hello\n");
}
