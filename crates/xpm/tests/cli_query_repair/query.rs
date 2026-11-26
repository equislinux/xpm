use super::*;

#[test]
fn query_exact_search_quiet_owner_and_foreign() {
    let fixture = Fixture::new();
    fixture.add_package("hello", "1.0-1");
    fixture.add_package("hello-world", "2.0-1");
    fixture.add_package("local-only", "3.0-1");

    fixture.write_files("hello", &["usr/", "usr/bin/", "usr/bin/hello"]);
    fixture.write_files("hello-world", &["usr/bin/hello-world"]);
    fixture.write_files("local-only", &["usr/bin/local-only"]);

    let exact = fixture.run(&["query", "hello"]);
    assert!(exact.status.success(), "{}", stderr(&exact));
    assert_eq!(stdout(&exact), "hello 1.0-1\n");

    let missing = fixture.run(&["query", "hell"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        stderr(&missing).contains("package 'hell' was not found"),
        "{}",
        stderr(&missing)
    );

    let search = fixture.run(&["query", "--search", "hello"]);
    assert!(search.status.success(), "{}", stderr(&search));
    let search_stdout = stdout(&search);
    assert!(search_stdout.contains("hello 1.0-1"));
    assert!(search_stdout.contains("hello-world 2.0-1"));

    let quiet = fixture.run(&["-Qq"]);
    assert!(quiet.status.success(), "{}", stderr(&quiet));
    let quiet_stdout = stdout(&quiet);
    for name in ["hello", "hello-world", "local-only"] {
        assert!(
            quiet_stdout.lines().any(|line| line == name),
            "missing quiet name {name}: {quiet_stdout}"
        );
    }
    assert!(!quiet_stdout.contains("1.0-1"), "{quiet_stdout}");

    let owner = fixture.run(&["-Qo", "/usr/bin/hello"]);
    assert!(owner.status.success(), "{}", stderr(&owner));
    assert_eq!(stdout(&owner), "/usr/bin/hello is owned by hello 1.0-1\n");

    let owner_quiet = fixture.run(&["-Qoq", "--", "/usr/bin/hello"]);
    assert!(owner_quiet.status.success(), "{}", stderr(&owner_quiet));
    assert_eq!(stdout(&owner_quiet), "hello\n");

    let owner_dir = fixture.run(&["query", "--owner", "usr/bin/"]);
    assert!(owner_dir.status.success(), "{}", stderr(&owner_dir));
    assert!(stdout(&owner_dir).contains("hello"));

    let unowned = fixture.run(&["-Qo", "/usr/bin/nope"]);
    assert_eq!(unowned.status.code(), Some(1));
    assert!(
        stderr(&unowned).contains("no package owns /usr/bin/nope"),
        "{}",
        stderr(&unowned)
    );

    fixture.write_sync_db("core", &[("hello", "1.0-1"), ("hello-world", "2.0-1")]);
    let foreign = fixture.run(&["-Qm"]);
    assert!(foreign.status.success(), "{}", stderr(&foreign));
    assert_eq!(stdout(&foreign), "local-only 3.0-1\n");

    let foreign_quiet = fixture.run(&["-Qmq"]);
    assert!(foreign_quiet.status.success(), "{}", stderr(&foreign_quiet));
    assert_eq!(stdout(&foreign_quiet), "local-only\n");
}

#[test]
fn query_unrequired_and_deps_modes_differ_from_orphans() {
    let fixture = Fixture::new();
    for name in ["app", "orphan-a", "orphan-b", "explicit-extra"] {
        fixture.add_package(name, "1.0-1");
    }
    fixture.write_reason("app", "explicit");
    fixture.write_reason("orphan-a", "dep");
    fixture.write_reason("orphan-b", "dep");
    fixture.write_reason("explicit-extra", "explicit");
    fixture.write_list("orphan-a", "depends", &["orphan-b"]);
    fixture.write_list("orphan-b", "depends", &[]);

    let unrequired = fixture.run(&["-Qt"]);
    assert!(unrequired.status.success(), "{}", stderr(&unrequired));
    let unrequired_stdout = stdout(&unrequired);
    assert!(
        unrequired_stdout.contains("app 1.0-1"),
        "{unrequired_stdout}"
    );
    assert!(
        unrequired_stdout.contains("explicit-extra 1.0-1"),
        "{unrequired_stdout}"
    );
    assert!(
        unrequired_stdout.contains("orphan-a 1.0-1"),
        "orphan-a is not required by anything else: {unrequired_stdout}"
    );
    assert!(
        !unrequired_stdout.contains("orphan-b 1.0-1"),
        "orphan-b is directly required by orphan-a: {unrequired_stdout}"
    );

    let deps = fixture.run(&["-Qdt"]);
    assert!(deps.status.success(), "{}", stderr(&deps));
    assert_eq!(stdout(&deps), "orphan-a 1.0-1\n");

    let orphans = fixture.run(&["query", "--orphans"]);
    assert!(orphans.status.success(), "{}", stderr(&orphans));
    let orphans_stdout = stdout(&orphans);
    assert!(orphans_stdout.contains("orphan-a"));
    assert!(
        orphans_stdout.contains("orphan-b"),
        "the transitive orphan mode is preserved: {orphans_stdout}"
    );

    let groups_quiet = fixture.run(&["-Qqg"]);
    assert!(groups_quiet.status.success(), "{}", stderr(&groups_quiet));
}
