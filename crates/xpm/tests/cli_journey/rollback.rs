use super::*;

#[test]
fn rollback_last_dry_run_and_execute_downgrade() {
    let fixture = Fixture::new();
    fixture.cache_package("hello", "0.9-1");
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);

    // Dry-run: plan only, database untouched.
    let dry = fixture.run(&["rollback", "--last", "--dry-run"]);
    assert!(
        dry.status.success(),
        "dry-run failed: {}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let stdout = String::from_utf8_lossy(&dry.stdout);
    assert!(
        stdout.contains("reinstall hello 0.9-1"),
        "unexpected dry-run output: {stdout}"
    );
    assert_eq!(fixture.read_db_version("hello"), "1.0-1");

    // Real rollback: the cached 0.9-1 must replace the installed 1.0-1.
    let run = fixture.run(&["rollback", "--last"]);
    assert!(
        run.status.success(),
        "rollback failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(fixture.read_db_version("hello"), "0.9-1");

    // A new journal entry documents the rollback.
    let history = fixture.run(&["history", "--json"]);
    let history_out = String::from_utf8_lossy(&history.stdout);
    assert!(
        history_out.contains("\"action\":\"rollback\""),
        "history must include the rollback: {history_out}"
    );
}

#[test]
fn rollback_reports_missing_cache_file() {
    let fixture = Fixture::new();
    fixture.install_into_db("hello", "1.0-1");
    fixture.record_journal(vec![JournalPackage::upgrade("hello", "0.9-1", "1.0-1")]);

    let run = fixture.run(&["rollback", "--last"]);
    assert!(!run.status.success(), "rollback must fail on a cache miss");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(
        stderr.contains("not in") && stderr.contains("hello 0.9-1"),
        "unexpected error: {stderr}"
    );
    // Nothing changed.
    assert_eq!(fixture.read_db_version("hello"), "1.0-1");
}
