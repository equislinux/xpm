use super::*;

#[test]
fn files_database_queries_and_refresh() {
    let fixture = Fixture::new();
    let hello = Entry::new("hello", "1.0-1");
    fixture.publish("core", std::slice::from_ref(&hello));

    let refresh = fixture.run(&["-Fy"]);
    assert_success(&refresh);

    let exact = fixture.run(&["-F", "usr/bin/hello"]);
    assert_success(&exact);
    assert_eq!(
        stdout(&exact),
        "usr/bin/hello is owned by core/hello 1.0-1\n"
    );

    let leading_slash = fixture.run(&["-F", "/usr/bin/hello"]);
    assert_success(&leading_slash);
    assert_eq!(
        stdout(&leading_slash),
        "/usr/bin/hello is owned by core/hello 1.0-1\n"
    );

    let quiet = fixture.run(&["-Fq", "usr/bin/hello"]);
    assert_success(&quiet);
    assert_eq!(stdout(&quiet), "core/hello\n");

    let list = fixture.run(&["-Fl", "hello"]);
    assert_success(&list);
    assert_eq!(stdout(&list), "hello usr/bin/hello\n");

    let missing = fixture.run(&["-F", "usr/bin/missing"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(stderr(&missing).contains("no package owns"));
}

#[test]
fn cache_clean_safe_and_all_modes() {
    let fixture = Fixture::new();
    fixture.add_installed("hello", "1.0-1");
    let entry = Entry::new("hello", "1.0-1");

    let current = fixture
        .cache_dir
        .join(format!("hello-1.0-1-{}.pkg.tar.zst", fixture.arch));
    let old = fixture
        .cache_dir
        .join(format!("hello-0.9-1-{}.pkg.tar.zst", fixture.arch));
    let orphan = fixture
        .cache_dir
        .join(format!("orphan-2.0-1-{}.pkg.tar.zst", fixture.arch));
    fs::write(&current, build_package(&fixture.arch, &entry)).expect("current");
    fs::write(
        &old,
        build_package(&fixture.arch, &Entry::new("hello", "0.9-1")),
    )
    .expect("old");
    fs::write(
        &orphan,
        build_package(&fixture.arch, &Entry::new("orphan", "2.0-1")),
    )
    .expect("orphan");
    fs::write(
        fixture
            .cache_dir
            .join(format!("broken-1.0-1-{}.pkg.tar.zst", fixture.arch)),
        b"garbage",
    )
    .expect("corrupt");
    fs::write(fixture.cache_dir.join("notes.txt"), b"keep").expect("notes");

    let safe = fixture.run(&["-Sc"]);
    assert_success(&safe);
    assert!(current.is_file(), "the installed version must stay cached");
    assert!(!old.exists());
    assert!(!orphan.exists());
    assert!(fixture
        .cache_dir
        .join(format!("broken-1.0-1-{}.pkg.tar.zst", fixture.arch))
        .is_file());
    assert!(fixture.cache_dir.join("notes.txt").is_file());
    assert!(stdout(&safe).contains("Removed 2 cached package file(s)"));

    let all = fixture.run(&["-Scc"]);
    assert_success(&all);
    assert!(!current.exists());
    assert!(fixture.cache_dir.join("notes.txt").is_file());
    assert!(fs::read_dir(&fixture.cache_dir)
        .expect("cache dir")
        .filter_map(|entry| entry.ok())
        .all(|entry| entry.file_name() == "notes.txt"));
}
