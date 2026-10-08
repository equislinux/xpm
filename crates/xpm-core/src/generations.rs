//! Read-only view of the X generations state.
//!
//! This module is the whole contract between xpm and the generations engine
//! (`x gen`, `equislinux/scripts`). xpm only *reads* the state; it never
//! creates snapshots, manifests or boot entries.
//!
//! Layout (see `docs/GENERATIONS.md`):
//!
//! ```text
//! <state>/current                    default generation id (e.g. `0004`)
//! <state>/generations/<id>/          manifest.json, packages.tsv, ...
//! ```
//!
//! `<state>` defaults to `<root_dir>/var/lib/x` and can be overridden with the
//! `X_GEN_STATE` environment variable (the engine does this too). The state
//! directory is root-only (0700): unreadable paths yield `None`/`Err` with a
//! clear message instead of a panic, so xpm keeps working without generations.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{XpmError, XpmResult};

/// Environment override for the generations state directory.
pub const ENV_STATE: &str = "X_GEN_STATE";
/// Default state directory relative to the installation root.
pub const DEFAULT_STATE_REL: &str = "var/lib/x";

const CURRENT_FILE: &str = "current";
const GENERATIONS_DIR: &str = "generations";
const PACKAGES_FILE: &str = "packages.tsv";

/// One `name version` pair captured in a generation's `packages.tsv`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GenerationPackage {
    pub name: String,
    pub version: String,
}

/// A version change between a generation capture and the live system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageChange {
    pub name: String,
    pub from: String,
    pub to: String,
}

/// Structured result of `xpm diff <generation>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GenerationDiff {
    pub generation: String,
    /// Installed now, absent from the capture.
    pub added: Vec<GenerationPackage>,
    /// Captured in the generation, absent now.
    pub removed: Vec<GenerationPackage>,
    /// Version changed between the capture and now.
    pub changed: Vec<PackageChange>,
}

impl GenerationDiff {
    /// True when the live system matches the capture exactly.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }

    /// Single-line JSON for machine consumption.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Compares the live `(name -> version)` map against a generation capture.
/// Output vectors are sorted by name for stable, diffable output.
pub fn diff_installed(
    installed: &HashMap<String, String>,
    captured: &[GenerationPackage],
    generation: &str,
) -> GenerationDiff {
    let mut captured_map: HashMap<String, String> = captured
        .iter()
        .map(|pkg| (pkg.name.clone(), pkg.version.clone()))
        .collect();

    let mut diff = GenerationDiff {
        generation: generation.to_string(),
        ..Default::default()
    };

    for (name, version) in installed {
        match captured_map.remove(name) {
            None => diff.added.push(GenerationPackage {
                name: name.clone(),
                version: version.clone(),
            }),
            Some(old) if old != *version => diff.changed.push(PackageChange {
                name: name.clone(),
                from: old,
                to: version.clone(),
            }),
            Some(_) => {}
        }
    }

    for (name, version) in captured_map {
        diff.removed.push(GenerationPackage { name, version });
    }

    diff.added.sort_by(|a, b| a.name.cmp(&b.name));
    diff.removed.sort_by(|a, b| a.name.cmp(&b.name));
    diff.changed.sort_by(|a, b| a.name.cmp(&b.name));
    diff
}

/// Resolves the state directory for `root_dir` (honoring `X_GEN_STATE`).
pub fn state_dir(root_dir: &Path) -> PathBuf {
    resolve_state_dir(root_dir, std::env::var_os(ENV_STATE).as_deref())
}

/// Pure part of [`state_dir`], split out so tests do not need the environment.
fn resolve_state_dir(root_dir: &Path, override_dir: Option<&OsStr>) -> PathBuf {
    match override_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => root_dir.join(DEFAULT_STATE_REL),
    }
}

/// Whether `id` is safe to join onto a path (digits normally, no traversal).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        // Reject `.`/`..`/`...`: at least one alphanumeric is always required.
        && id.bytes().any(|b| b.is_ascii_alphanumeric())
}

/// Reads `<state>/current` and returns the id when it is valid.
///
/// Missing/unreadable state (no generations, root-only directory) is `None`:
/// linking a journal to a generation is best-effort by design.
pub fn read_current(root_dir: &Path) -> Option<String> {
    let path = state_dir(root_dir).join(CURRENT_FILE);
    let raw = fs::read_to_string(&path).ok()?;
    let id = raw.lines().next()?.trim().to_string();
    if valid_id(&id) {
        Some(id)
    } else {
        tracing::debug!(path = %path.display(), "ignoring malformed generation id");
        None
    }
}

/// Directory holding a generation's metadata, if `id` is safe.
pub fn generation_dir(root_dir: &Path, id: &str) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    Some(state_dir(root_dir).join(GENERATIONS_DIR).join(id))
}

/// Reads and parses the `packages.tsv` capture of a generation.
///
/// The file is `pacman -Q` style: one `name version` pair per line, sorted.
pub fn read_generation_packages(root_dir: &Path, id: &str) -> XpmResult<Vec<GenerationPackage>> {
    let dir = generation_dir(root_dir, id).ok_or_else(|| {
        XpmError::Other(format!(
            "invalid generation id '{id}' (expected alphanumerics, '.', '_' or '-')"
        ))
    })?;
    let path = dir.join(PACKAGES_FILE);

    match fs::read_to_string(&path) {
        Ok(raw) => Ok(parse_packages_tsv(&raw)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(XpmError::Other(format!(
            "generation '{id}' has no package capture at {}",
            path.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(XpmError::Other(
            "generation state is root-only (/var/lib/x); re-run with sudo".to_string(),
        )),
        Err(e) => Err(e.into()),
    }
}

/// Parses `pacman -Q`/`xpm query` style output. Blank lines are skipped and
/// extra columns are ignored (the first two fields win).
pub fn parse_packages_tsv(raw: &str) -> Vec<GenerationPackage> {
    raw.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let name = fields.next()?;
            let version = fields.next().unwrap_or_default();
            Some(GenerationPackage {
                name: name.to_string(),
                version: version.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_state(root: &Path, current: &str, id: &str, packages: &str) -> PathBuf {
        let state = root.join(DEFAULT_STATE_REL);
        fs::create_dir_all(state.join(GENERATIONS_DIR).join(id)).expect("create state");
        fs::write(state.join(CURRENT_FILE), format!("{current}\n")).expect("write current");
        let tsv = state.join(GENERATIONS_DIR).join(id).join(PACKAGES_FILE);
        fs::write(&tsv, packages).expect("write packages.tsv");
        tsv
    }

    #[test]
    fn state_dir_defaults_to_root_relative() {
        let root = Path::new("/mnt/x");
        assert_eq!(
            resolve_state_dir(root, None),
            PathBuf::from("/mnt/x/var/lib/x")
        );
        assert_eq!(
            resolve_state_dir(root, Some(OsStr::new("/custom/state"))),
            PathBuf::from("/custom/state")
        );
        assert_eq!(
            resolve_state_dir(root, Some(OsStr::new(""))),
            PathBuf::from("/mnt/x/var/lib/x"),
            "empty override falls back to the root-relative default"
        );
    }

    #[test]
    fn reads_current_and_packages() {
        // `read_current` uses the environment; write the state under a temp
        // root and point the function there through X_GEN_STATE is racy in
        // parallel tests, so exercise the pieces directly.
        let tmp = TempDir::new().expect("tmp");
        let tsv = write_state(
            tmp.path(),
            "0003",
            "0003",
            "bash 5.2.037-1\nkitty 0.44.0-1\n",
        );
        assert!(tsv.exists());

        let packages = parse_packages_tsv(&fs::read_to_string(&tsv).expect("read"));
        assert_eq!(
            packages,
            vec![
                GenerationPackage {
                    name: "bash".into(),
                    version: "5.2.037-1".into()
                },
                GenerationPackage {
                    name: "kitty".into(),
                    version: "0.44.0-1".into()
                },
            ]
        );
    }

    #[test]
    fn parse_skips_blanks_and_ignores_extra_columns() {
        let packages = parse_packages_tsv("\nfoo 1.0-1 extra\n\nbar 2.0-1\n");
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].version, "1.0-1");
    }

    #[test]
    fn rejects_traversal_ids() {
        assert!(!valid_id(".."));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"x".repeat(65)));
        assert!(valid_id("0001"));
        assert!(valid_id("0002-pinned"));
        assert!(generation_dir(Path::new("/"), "../etc").is_none());
    }

    #[test]
    fn diff_classifies_added_removed_and_changed() {
        let installed: HashMap<String, String> = [
            ("bash".to_string(), "5.3.0-1".to_string()),
            ("kitty".to_string(), "0.44.0-1".to_string()),
            ("new".to_string(), "1.0-1".to_string()),
        ]
        .into_iter()
        .collect();
        let captured = vec![
            GenerationPackage {
                name: "bash".into(),
                version: "5.2.0-1".into(),
            },
            GenerationPackage {
                name: "kitty".into(),
                version: "0.44.0-1".into(),
            },
            GenerationPackage {
                name: "old".into(),
                version: "1.0-1".into(),
            },
        ];

        let diff = diff_installed(&installed, &captured, "0003");
        assert_eq!(diff.generation, "0003");
        assert!(!diff.is_empty());
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].name, "new");
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].name, "old");
        assert_eq!(diff.changed.len(), 1);
        assert_eq!(diff.changed[0].from, "5.2.0-1");
        assert_eq!(diff.changed[0].to, "5.3.0-1");
    }

    #[test]
    fn diff_is_empty_when_identical_and_json_roundtrips() {
        let installed: HashMap<String, String> = [("kitty".to_string(), "0.44.0-1".to_string())]
            .into_iter()
            .collect();
        let captured = vec![GenerationPackage {
            name: "kitty".into(),
            version: "0.44.0-1".into(),
        }];
        let diff = diff_installed(&installed, &captured, "0001");
        assert!(diff.is_empty());

        let json = diff.to_json();
        assert!(json.contains("\"generation\":\"0001\""));
        assert!(json.contains("\"added\":[]"));
    }

    #[test]
    fn missing_capture_reports_clear_error() {
        let tmp = TempDir::new().expect("tmp");
        let err = read_generation_packages(tmp.path(), "0001").expect_err("must fail");
        let message = err.to_string();
        assert!(
            message.contains("no package capture"),
            "unexpected message: {message}"
        );
    }
}
