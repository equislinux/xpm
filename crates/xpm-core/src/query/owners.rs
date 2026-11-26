//! Owner lookup (`-Qo`).

use super::*;

/// One package that owns a queried path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub name: String,
    pub version: String,
}

/// Normalizes an owner query (absolute or relative) to a manifest relative
/// path. The configured root is stripped when the query lives below it; `..`
/// never escapes the root.
pub fn normalize_query_path(query: &str, root: &Path) -> Option<String> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    let path = Path::new(query);
    let relative = if path.is_absolute() {
        path.strip_prefix(root).unwrap_or(path)
    } else {
        path
    };

    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn normalize_manifest_entry(entry: &str) -> Option<String> {
    let mut entry = entry.trim();
    while let Some(stripped) = entry.strip_prefix("./") {
        entry = stripped;
    }
    let entry = entry.trim_start_matches('/').trim_end_matches('/');
    if entry.is_empty() {
        None
    } else {
        Some(entry.to_string())
    }
}

fn manifest_entry_matches(entry: &str, normalized: &str, basename_only: bool) -> bool {
    let Some(entry) = normalize_manifest_entry(entry) else {
        return false;
    };

    if entry == normalized {
        return true;
    }

    if basename_only {
        Path::new(&entry)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == normalized)
    } else {
        false
    }
}

/// Finds the installed packages whose `files` manifest contains `query`.
///
/// Both absolute and relative paths are accepted. A path with a trailing
/// slash still matches the directory entry, and a bare basename (no `/`) is
/// matched against every manifest entry's final component.
pub fn find_owners(packages: &[LocalPackage], root: &Path, query: &str) -> Vec<Owner> {
    let Some(normalized) = normalize_query_path(query, root) else {
        return Vec::new();
    };
    let basename_only = !normalized.contains('/');

    let mut owners: Vec<Owner> = packages
        .iter()
        .filter(|package| {
            package
                .files
                .iter()
                .any(|entry| manifest_entry_matches(entry, &normalized, basename_only))
        })
        .map(|package| Owner {
            name: package.name.clone(),
            version: package.version.clone(),
        })
        .collect();

    owners.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));
    owners
}
