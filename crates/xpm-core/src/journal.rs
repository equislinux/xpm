//! Persistent transaction journal.
//!
//! Every transaction is recorded as a JSON file under the journal directory
//! (default: `<db_path>/journal`). The file is written before touching the
//! filesystem and finalized after the commit, so a crash leaves a `running`
//! entry with the intended operations. `xpm history` reads it back and the
//! provisioning payload (`x gen`) consumes it to align generations with xpm
//! transactions. See `docs/GENERATIONS.md`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{XpmError, XpmResult};

pub const JOURNAL_SCHEMA: u32 = 1;

/// One package affected by a transaction.
///
/// `repo`, `sha256` and `source` are provenance fields filled when the package
/// comes from a repository. They are all optional so schema-1 journals written
/// before they existed keep parsing unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalPackage {
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
    /// Repository the package was downloaded from (`None` for local files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// SHA-256 of the downloaded package file, when the sync DB declared one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Mirror URL the package was downloaded from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl JournalPackage {
    pub fn install(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from: None,
            to: Some(version.into()),
            repo: None,
            sha256: None,
            source: None,
        }
    }

    pub fn remove(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from: Some(version.into()),
            to: None,
            repo: None,
            sha256: None,
            source: None,
        }
    }

    pub fn upgrade(
        name: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            from: Some(from.into()),
            to: Some(to.into()),
            repo: None,
            sha256: None,
            source: None,
        }
    }

    /// Attaches repository provenance to the entry.
    pub fn with_repo(mut self, repo: impl Into<String>) -> Self {
        self.repo = Some(repo.into());
        self
    }

    /// Attaches the downloaded file checksum to the entry.
    pub fn with_sha256(mut self, sha256: impl Into<String>) -> Self {
        self.sha256 = Some(sha256.into());
        self
    }

    /// Attaches the download source URL to the entry.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Human-readable form: `name 1.0-1`, `name 1.0-1 -> 1.1-1` or
    /// `name 1.0-1 (removed)`.
    pub fn describe(&self) -> String {
        match (&self.from, &self.to) {
            (Some(from), Some(to)) => format!("{} {from} -> {to}", self.name),
            (None, Some(to)) => format!("{} {to}", self.name),
            (Some(from), None) => format!("{} {from} (removed)", self.name),
            (None, None) => self.name.clone(),
        }
    }
}

/// A full transaction record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Journal {
    pub schema: u32,
    pub id: String,
    pub action: String,
    pub root_dir: String,
    pub started: u64,
    pub finished: Option<u64>,
    /// `running`, `ok` or `failed`.
    pub result: String,
    pub packages: Vec<JournalPackage>,
    pub error: Option<String>,
    /// Generation id active when the transaction was finalized (read from the
    /// generations state directory; `None` when generations are unavailable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    #[serde(skip)]
    pub path: PathBuf,
}

impl Journal {
    /// Creates and persists a `running` journal entry.
    pub fn start(
        dir: &Path,
        action: &str,
        root_dir: &Path,
        packages: Vec<JournalPackage>,
    ) -> XpmResult<Self> {
        fs::create_dir_all(dir)?;
        let started = now_epoch();
        let id = format!("{started}-{}", std::process::id());
        let path = dir.join(format!("{id}.json"));
        let journal = Self {
            schema: JOURNAL_SCHEMA,
            id,
            action: action.to_string(),
            root_dir: root_dir.display().to_string(),
            started,
            finished: None,
            result: "running".to_string(),
            packages,
            error: None,
            generation: None,
            path,
        };
        journal.persist()?;
        Ok(journal)
    }

    /// Finalizes the entry with a result (`ok`/`failed`) and persists it.
    pub fn finish(&mut self, result: &str, error: Option<String>) -> XpmResult<()> {
        self.result = result.to_string();
        self.error = error;
        self.finished = Some(now_epoch());
        self.persist()
    }

    pub fn persist(&self) -> XpmResult<()> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| XpmError::Other(format!("failed to serialize journal: {e}")))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> XpmResult<Self> {
        let data = fs::read_to_string(path)?;
        let mut journal: Journal = serde_json::from_str(&data)
            .map_err(|e| XpmError::Other(format!("invalid journal {}: {e}", path.display())))?;
        journal.path = path.to_path_buf();
        Ok(journal)
    }

    /// Reads every journal file in `dir`, newest first, ignoring unreadable
    /// entries.
    pub fn list(dir: &Path) -> XpmResult<Vec<Journal>> {
        let mut out = Vec::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(journal) = Self::load(&path) {
                out.push(journal);
            }
        }
        out.sort_by(|a, b| b.started.cmp(&a.started).then_with(|| b.id.cmp(&a.id)));
        Ok(out)
    }

    /// Single-line human summary for `xpm history`. The generation marker is
    /// appended only when the journal knows which generation it produced.
    pub fn summary(&self) -> String {
        let pkgs = self
            .packages
            .iter()
            .map(JournalPackage::describe)
            .collect::<Vec<_>>()
            .join(", ");
        let generation = match &self.generation {
            Some(id) => format!("  gen:{id}"),
            None => String::new(),
        };
        format!(
            "{}  {:<7}  {:<7}  {:>3} pkg  {}{}",
            iso8601(self.started),
            self.result,
            self.action,
            self.packages.len(),
            pkgs,
            generation
        )
    }

    /// Compact JSON line (for `xpm history --json`).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// UTC timestamp from an epoch, `YYYY-MM-DDTHH:MM:SSZ`
/// (civil-from-days algorithm, no external crates).
pub fn iso8601(epoch: u64) -> String {
    let days = (epoch / 86_400) as i64;
    let secs = epoch % 86_400;
    let (h, mi, s) = (secs / 3_600, (secs % 3_600) / 60, secs % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
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
}
