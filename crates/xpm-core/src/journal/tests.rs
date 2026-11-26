use super::*;
use tempfile::TempDir;

#[test]
fn start_persists_running_entry() {
    let tmp = TempDir::new().expect("tmp");
    let journal = Journal::start(
        tmp.path(),
        "install",
        Path::new("/"),
        vec![JournalPackage::install("kitty", "0.44.0-1")],
    )
    .expect("start");

    assert_eq!(journal.result, "running");
    assert!(journal.path.exists());
    let loaded = Journal::load(&journal.path).expect("load");
    assert_eq!(loaded, journal);
    assert_eq!(loaded.packages[0].name, "kitty");
}

#[test]
fn finish_marks_ok_and_failure() {
    let tmp = TempDir::new().expect("tmp");
    let mut journal =
        Journal::start(tmp.path(), "upgrade", Path::new("/"), Vec::new()).expect("start");
    journal.finish("ok", None).expect("finish");
    let loaded = Journal::load(&journal.path).expect("load");
    assert_eq!(loaded.result, "ok");
    assert!(loaded.finished.is_some());

    let mut failed = Journal::start(
        tmp.path(),
        "remove",
        Path::new("/"),
        vec![JournalPackage::remove("foo", "1.0-1")],
    )
    .expect("start");
    failed
        .finish("failed", Some("boom".to_string()))
        .expect("finish");
    let loaded = Journal::load(&failed.path).expect("load");
    assert_eq!(loaded.result, "failed");
    assert_eq!(loaded.error.as_deref(), Some("boom"));
}

#[test]
fn list_is_newest_first_and_ignores_garbage() {
    let tmp = TempDir::new().expect("tmp");
    let mut a = Journal::start(tmp.path(), "install", Path::new("/"), Vec::new()).expect("a");
    // Force a distinct, older timestamp.
    a.started = 1_000;
    a.id = "1000-1".to_string();
    a.path = tmp.path().join("1000-1.json");
    a.persist().expect("persist");
    let b = Journal::start(tmp.path(), "remove", Path::new("/"), Vec::new()).expect("b");
    fs::write(tmp.path().join("garbage.json"), "not json").expect("garbage");
    fs::write(tmp.path().join("notes.txt"), "ignore me").expect("txt");

    let list = Journal::list(tmp.path()).expect("list");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, b.id);
    assert_eq!(list[1].id, "1000-1");
}

#[test]
fn iso8601_known_epochs() {
    assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso8601(946_684_800), "2000-01-01T00:00:00Z");
    assert_eq!(iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
}

#[test]
fn legacy_schema1_journal_parses_without_new_fields() {
    let tmp = TempDir::new().expect("tmp");
    let path = tmp.path().join("legacy.json");
    // A journal exactly as written by older xpm versions: no `repo`,
    // `sha256`, `source` or `generation` keys anywhere.
    fs::write(
        &path,
        r#"{
                "schema": 1,
                "id": "1000-1",
                "action": "install",
                "root_dir": "/",
                "started": 1000,
                "finished": 1001,
                "result": "ok",
                "packages": [{"name": "kitty", "from": null, "to": "0.44.0-1"}],
                "error": null
            }"#,
    )
    .expect("write legacy journal");

    let journal = Journal::load(&path).expect("legacy journal must load");
    assert_eq!(journal.packages[0].repo, None);
    assert_eq!(journal.packages[0].sha256, None);
    assert_eq!(journal.packages[0].source, None);
    assert_eq!(journal.generation, None);
}

#[test]
fn provenance_and_generation_roundtrip() {
    let tmp = TempDir::new().expect("tmp");
    let mut journal = Journal::start(
        tmp.path(),
        "install",
        Path::new("/"),
        vec![JournalPackage::install("kitty", "0.44.0-1")
            .with_repo("x")
            .with_sha256("abc123")
            .with_source("https://example.com/kitty.pkg.tar.zst")],
    )
    .expect("start");
    journal.generation = Some("0007".to_string());
    journal.persist().expect("persist");

    let loaded = Journal::load(&journal.path).expect("load");
    assert_eq!(loaded.packages[0].repo.as_deref(), Some("x"));
    assert_eq!(loaded.packages[0].sha256.as_deref(), Some("abc123"));
    assert_eq!(
        loaded.packages[0].source.as_deref(),
        Some("https://example.com/kitty.pkg.tar.zst")
    );
    assert_eq!(loaded.generation.as_deref(), Some("0007"));

    let json = loaded.to_json();
    assert!(json.contains("\"generation\":\"0007\""));
    assert!(json.contains("\"repo\":\"x\""));
}

#[test]
fn summary_appends_generation_only_when_known() {
    let tmp = TempDir::new().expect("tmp");
    let mut journal =
        Journal::start(tmp.path(), "install", Path::new("/"), Vec::new()).expect("journal");
    assert!(!journal.summary().contains("gen:"));

    journal.generation = Some("0002".to_string());
    assert!(journal.summary().contains("gen:0002"));
}

#[test]
fn package_describe() {
    assert_eq!(
        JournalPackage::install("kitty", "1.0-1").describe(),
        "kitty 1.0-1"
    );
    assert_eq!(
        JournalPackage::upgrade("kitty", "1.0-1", "1.1-1").describe(),
        "kitty 1.0-1 -> 1.1-1"
    );
    assert_eq!(
        JournalPackage::remove("kitty", "1.0-1").describe(),
        "kitty 1.0-1 (removed)"
    );
}
