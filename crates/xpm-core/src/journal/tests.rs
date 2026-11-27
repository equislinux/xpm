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

use crate::local_db;

fn write_test_package(
    cache: &Path,
    name: &str,
    version: &str,
    depends: &[&str],
    provides: &[&str],
) -> PathBuf {
    let path = cache.join(format!("{name}-{version}-x86_64.pkg.tar.zst"));
    let file = fs::File::create(&path).expect("create package");
    let mut encoder = zstd::stream::Encoder::new(file, 3).expect("encoder");
    {
        let mut tar = tar::Builder::new(&mut encoder);
        let mut pkginfo = format!("pkgname = {name}\npkgver = {version}\n");
        for dependency in depends {
            pkginfo.push_str(&format!("depend = {dependency}\n"));
        }
        for provide in provides {
            pkginfo.push_str(&format!("provides = {provide}\n"));
        }
        let mut header = tar::Header::new_gnu();
        header.set_size(pkginfo.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, ".PKGINFO", pkginfo.as_bytes())
            .expect("append .PKGINFO");
        tar.finish().expect("finish tar");
    }
    encoder.finish().expect("finish zstd");
    path
}

#[test]
fn reconcile_running_rewrites_stale_metadata_from_cache() {
    let tmp = TempDir::new().expect("tmp");
    let db = tmp.path().join("db");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(db.join("local/stale")).expect("local db");
    fs::create_dir_all(&cache).expect("cache");
    fs::write(db.join("local/stale/version"), "2.0-1").expect("version");
    fs::write(db.join("local/stale/provides"), "libstale.so=1-64\n").expect("provides");
    fs::write(db.join("local/stale/depends"), "glibc\n").expect("depends");

    write_test_package(&cache, "stale", "2.0-1", &["glibc"], &["libstale.so=2-64"]);
    let journal = Journal::start(
        &db.join("journal"),
        "upgrade",
        Path::new("/"),
        vec![JournalPackage::upgrade("stale", "1.0-1", "2.0-1")],
    )
    .expect("journal");

    let report = reconcile_running(&db, &cache).expect("reconcile");
    assert_eq!(report.reconciled, vec!["stale 2.0-1"]);
    assert!(report.unresolved.is_empty());
    assert_eq!(
        local_db::read_provides(&db.join("local"), "stale").expect("read provides"),
        vec!["libstale.so=2-64".to_string()]
    );
    assert_eq!(
        local_db::read_depends(&db.join("local"), "stale").expect("read depends"),
        Some(vec!["glibc".to_string()])
    );

    let loaded = Journal::load(&journal.path).expect("load journal");
    assert_eq!(loaded.result, "recovered");
}

#[test]
fn reconcile_running_reports_missing_cached_package() {
    let tmp = TempDir::new().expect("tmp");
    let db = tmp.path().join("db");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(db.join("local/stale")).expect("local db");
    fs::create_dir_all(&cache).expect("cache");
    fs::write(db.join("local/stale/version"), "2.0-1").expect("version");
    fs::write(db.join("local/stale/provides"), "libstale.so=1-64\n").expect("provides");

    let journal = Journal::start(
        &db.join("journal"),
        "upgrade",
        Path::new("/"),
        vec![JournalPackage::upgrade("stale", "1.0-1", "2.0-1")],
    )
    .expect("journal");

    let report = reconcile_running(&db, &cache).expect("reconcile");
    assert!(report.reconciled.is_empty());
    assert_eq!(report.unresolved.len(), 1);
    assert!(report.unresolved[0].contains("package not in cache"));
    assert_eq!(
        local_db::read_provides(&db.join("local"), "stale").expect("read provides"),
        vec!["libstale.so=1-64".to_string()],
        "the stale record must be left untouched"
    );

    let loaded = Journal::load(&journal.path).expect("load journal");
    assert_eq!(loaded.result, "failed");
    assert!(loaded.error.is_some());
}

#[test]
fn reconcile_running_skips_uncommitted_versions() {
    let tmp = TempDir::new().expect("tmp");
    let db = tmp.path().join("db");
    let cache = tmp.path().join("cache");
    fs::create_dir_all(db.join("local/stale")).expect("local db");
    fs::create_dir_all(&cache).expect("cache");
    fs::write(db.join("local/stale/version"), "1.0-1").expect("version");
    fs::write(db.join("local/stale/provides"), "libstale.so=1-64\n").expect("provides");

    write_test_package(&cache, "stale", "2.0-1", &[], &["libstale.so=2-64"]);
    let journal = Journal::start(
        &db.join("journal"),
        "upgrade",
        Path::new("/"),
        vec![JournalPackage::upgrade("stale", "1.0-1", "2.0-1")],
    )
    .expect("journal");

    let report = reconcile_running(&db, &cache).expect("reconcile");
    assert!(report.reconciled.is_empty());
    assert_eq!(
        local_db::read_provides(&db.join("local"), "stale").expect("read provides"),
        vec!["libstale.so=1-64".to_string()],
        "untouched record because the new version was never recorded"
    );

    let loaded = Journal::load(&journal.path).expect("load journal");
    assert_eq!(loaded.result, "recovered");
}
