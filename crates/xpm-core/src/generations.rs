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
mod tests;
