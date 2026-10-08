//! Rollback planning for xpm transactions.
//!
//! `xpm rollback --last` is the *package-level* recovery path: it computes the
//! inverse of the newest successful journal entry and replays it as a new
//! transaction, using the local package cache for the old versions.
//!
//! Boundaries (see `docs/GENERATIONS.md`):
//!
//! - xpm never touches btrfs snapshots, boot entries or `/var/lib/x`. Whole
//!   system recovery remains `x gen rollback`.
//! - If an old package file is no longer cached, the plan reports it as
//!   missing instead of guessing; the caller aborts before changing anything.

use std::path::{Path, PathBuf};

use crate::cache::find_package;
use crate::error::XpmResult;
use crate::journal::Journal;
use crate::local_db;

/// One inverse operation of a journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RollbackOp {
    /// Undo of an install: remove the package (only if still installed).
    Remove {
        name: String,
        version: Option<String>,
    },
    /// Undo of a remove/upgrade: reinstall `version` from the package cache.
    Reinstall {
        name: String,
        version: String,
        file: PathBuf,
    },
}

impl RollbackOp {
    /// Single-line human description for `--dry-run` output.
    pub fn describe(&self) -> String {
        match self {
            RollbackOp::Remove { name, version } => match version {
                Some(version) => format!("remove {name} {version}"),
                None => format!("remove {name}"),
            },
            RollbackOp::Reinstall {
                name,
                version,
                file,
            } => format!("reinstall {name} {version} ({})", file.display()),
        }
    }
}

/// Everything needed to execute (or explain) a rollback.
#[derive(Debug, Clone, Default)]
pub struct RollbackPlan {
    /// Journal being undone.
    pub journal_id: String,
    /// Action of that journal (`install`, `remove` or `upgrade`).
    pub action: String,
    /// Generation linked to that journal, when known.
    pub generation: Option<String>,
    pub ops: Vec<RollbackOp>,
    /// Packages whose old file is not in the cache (blocking).
    pub missing: Vec<(String, String)>,
    /// Entries that no longer apply (informational).
    pub skipped: Vec<String>,
}

impl RollbackPlan {
    /// True when there is nothing to do.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty() && self.missing.is_empty()
    }

    /// Whether the plan can be executed (all files available).
    pub fn is_executable(&self) -> bool {
        self.missing.is_empty()
    }

    /// Multi-line summary used by `xpm rollback --dry-run`.
    pub fn describe(&self) -> String {
        let mut out = format!("rollback of {} (action: {})", self.journal_id, self.action);
        if let Some(generation) = &self.generation {
            out.push_str(&format!(" [gen:{generation}]"));
        }
        if self.ops.is_empty() && self.missing.is_empty() {
            out.push_str("\n  nothing to undo");
        }
        for op in &self.ops {
            out.push_str("\n  ");
            out.push_str(&op.describe());
        }
        for (name, version) in &self.missing {
            out.push_str(&format!("\n  MISSING {name} {version} (not in cache)"));
        }
        for skipped in &self.skipped {
            out.push_str(&format!("\n  skipped {skipped}"));
        }
        out
    }
}

/// Newest successful journal eligible for rollback, if any.
///
/// Failed or still-running entries are ignored: rolling back a half-applied
/// transaction automatically is not safe; `xpm history` shows those.
pub fn last_rollback_candidate(journals: &[Journal]) -> Option<&Journal> {
    journals.iter().find(|journal| {
        journal.result == "ok"
            && matches!(journal.action.as_str(), "install" | "remove" | "upgrade")
    })
}

/// Computes the inverse plan of `journal` using the local database and the
/// package cache. Pure (no filesystem writes), so it is cheap to test.
pub fn build_plan(
    journal: &Journal,
    local_db_dir: &Path,
    cache_dir: &Path,
) -> XpmResult<RollbackPlan> {
    let mut plan = RollbackPlan {
        journal_id: journal.id.clone(),
        action: journal.action.clone(),
        generation: journal.generation.clone(),
        ..Default::default()
    };

    for pkg in &journal.packages {
        match (&pkg.from, &pkg.to) {
            // The journal installed it: undo = remove, if still present.
            (None, Some(_)) => match local_db::read_version(local_db_dir, &pkg.name) {
                Some(version) => plan.ops.push(RollbackOp::Remove {
                    name: pkg.name.clone(),
                    version: Some(version),
                }),
                None => plan.skipped.push(format!("{} (already absent)", pkg.name)),
            },
            // The journal removed or upgraded it: undo = reinstall the old file.
            (Some(from), _) => match find_package(cache_dir, &pkg.name, from)? {
                Some(file) => plan.ops.push(RollbackOp::Reinstall {
                    name: pkg.name.clone(),
                    version: from.clone(),
                    file,
                }),
                None => plan.missing.push((pkg.name.clone(), from.clone())),
            },
            (None, None) => {}
        }
    }

    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::JournalPackage;
    use std::fs;
    use tempfile::TempDir;

    fn journal_with(action: &str, packages: Vec<JournalPackage>) -> Journal {
        let tmp = TempDir::new().expect("tmp");
        let mut journal =
            Journal::start(tmp.path(), action, Path::new("/"), packages).expect("journal");
        journal.result = "ok".to_string();
        journal.finish("ok", None).expect("finish");
        journal
    }

    #[test]
    fn selects_newest_successful_relevant_entry() {
        let mut newer_failed = journal_with("install", Vec::new());
        newer_failed.result = "failed".to_string();
        let newer_ok = journal_with("upgrade", Vec::new());
        let older_ok = journal_with("install", Vec::new());
        let journals = [newer_failed, newer_ok.clone(), older_ok];
        let selected = last_rollback_candidate(&journals).expect("candidate");
        assert_eq!(selected.id, newer_ok.id);
    }

    #[test]
    fn install_is_undone_with_remove_when_still_present() {
        let db = TempDir::new().expect("db");
        let cache = TempDir::new().expect("cache");
        let pkg_dir = db.path().join("kitty");
        fs::create_dir_all(&pkg_dir).expect("pkg dir");
        fs::write(pkg_dir.join("version"), "0.44.0-1").expect("version");

        let journal = journal_with(
            "install",
            vec![JournalPackage::install("kitty", "0.44.0-1")],
        );
        let plan = build_plan(&journal, db.path(), cache.path()).expect("plan");
        assert!(plan.is_executable());
        assert_eq!(
            plan.ops,
            vec![RollbackOp::Remove {
                name: "kitty".into(),
                version: Some("0.44.0-1".into())
            }]
        );
    }

    #[test]
    fn removed_package_is_reinstalled_from_cache_and_missing_is_reported() {
        let db = TempDir::new().expect("db");
        let cache = TempDir::new().expect("cache");
        // No cached file exists: both removals must be reported missing.
        let journal = journal_with(
            "remove",
            vec![
                JournalPackage::remove("foo", "1.0-1"),
                JournalPackage::remove("bar", "2.0-1"),
            ],
        );
        let plan = build_plan(&journal, db.path(), cache.path()).expect("plan");
        assert!(!plan.is_executable());
        assert_eq!(
            plan.missing,
            vec![
                ("foo".to_string(), "1.0-1".to_string()),
                ("bar".to_string(), "2.0-1".to_string())
            ]
        );
        assert!(plan.describe().contains("MISSING foo 1.0-1"));
    }

    #[test]
    fn already_absent_install_is_skipped_not_failed() {
        let db = TempDir::new().expect("db");
        let cache = TempDir::new().expect("cache");
        let journal = journal_with(
            "install",
            vec![JournalPackage::install("kitty", "0.44.0-1")],
        );
        let plan = build_plan(&journal, db.path(), cache.path()).expect("plan");
        assert!(plan.is_empty());
        assert!(plan.describe().contains("skipped kitty"));
    }
}
