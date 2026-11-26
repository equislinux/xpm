use super::*;

#[test]
fn rollback_runs_alpm_hooks_in_order() {
    let fixture = Fixture::new();
    fixture.cache_package("hello", "0.9-1");
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);

    let marker = fixture._tmp.path().join("hook-order.txt");
    fixture.add_alpm_hook("10-pre", "PreTransaction", &marker, "pre");
    fixture.add_alpm_hook("20-post", "PostTransaction", &marker, "post");

    let run = fixture.run(&["rollback", "--last"]);
    assert!(
        run.status.success(),
        "rollback with hooks failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let order = fs::read_to_string(&marker).expect("hook marker");
    assert_eq!(order, "pre\npost\n");
    assert_eq!(fixture.read_db_version("hello"), "0.9-1");
}

#[test]
fn no_alpm_hooks_flag_skips_hooks() {
    let fixture = Fixture::new();
    fixture.cache_package("hello", "0.9-1");
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);

    let marker = fixture._tmp.path().join("hook-order.txt");
    fixture.add_alpm_hook("10-pre", "PreTransaction", &marker, "pre");
    fixture.add_alpm_hook("20-post", "PostTransaction", &marker, "post");

    let run = fixture.run(&["--no-alpm-hooks", "rollback", "--last"]);
    assert!(
        run.status.success(),
        "rollback with hooks disabled failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        !marker.exists(),
        "ALPM hooks must not run with --no-alpm-hooks"
    );
    assert_eq!(fixture.read_db_version("hello"), "0.9-1");
}

#[test]
fn alpm_hooks_false_config_skips_hooks() {
    let fixture = Fixture::new();
    fixture.cache_package("hello", "0.9-1");
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);
    fixture.append_config("alpm_hooks = false\n");

    let marker = fixture._tmp.path().join("hook-order.txt");
    fixture.add_alpm_hook("10-pre", "PreTransaction", &marker, "pre");
    fixture.add_alpm_hook("20-post", "PostTransaction", &marker, "post");

    let run = fixture.run(&["rollback", "--last"]);
    assert!(
        run.status.success(),
        "rollback with alpm_hooks=false failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        !marker.exists(),
        "ALPM hooks must not run with alpm_hooks=false"
    );
}
