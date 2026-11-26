//! Journal records, persistence and history listing.

use super::*;

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
