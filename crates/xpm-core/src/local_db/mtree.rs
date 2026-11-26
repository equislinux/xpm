//! `.MTREE` conversions for the local database.

use super::*;

/// Converts `.MTREE` entries into pacman-compatible relative paths.
///
/// The `./` prefix is stripped, directories get a trailing `/` and order is
/// preserved (`.MTREE` is sorted). Duplicates and the root entry are skipped.
pub fn mtree_paths(entries: &[MtreeEntry]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut paths = Vec::with_capacity(entries.len());

    for entry in entries {
        let Some(mut rel) = normalize_mtree_path(&entry.path.to_string_lossy()) else {
            continue;
        };
        if is_package_metadata_path(&rel) {
            continue;
        }
        if entry.file_type == MtreeFileType::Dir && !rel.ends_with('/') {
            rel.push('/');
        }
        if seen.insert(rel.clone()) {
            paths.push(rel);
        }
    }

    paths
}

/// Whether a normalized path is package metadata. Real Arch packages include
/// `.PKGINFO`/`.BUILDINFO`/`.MTREE` themselves in the `.MTREE`, and these must
/// never leak into the installed-file manifest (removal would target them).
fn is_package_metadata_path(path: &str) -> bool {
    matches!(path, ".PKGINFO" | ".BUILDINFO" | ".MTREE" | ".INSTALL")
}

fn normalize_mtree_path(raw: &str) -> Option<String> {
    let mut parts = Vec::new();

    for component in Path::new(raw).components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::RootDir | Component::CurDir => {}
            // Never emit a path that escapes the target root.
            Component::ParentDir => return None,
            Component::Prefix(_) => {}
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("/"))
}

/// Maps normalized paths to their recorded SHA-256 for regular files.
///
/// Used to answer "was this configuration file modified since install?" by
/// comparing the on-disk hash with the one captured in the old `.MTREE`.
pub fn mtree_hash_map(entries: &[MtreeEntry]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for entry in entries {
        if entry.file_type != MtreeFileType::File {
            continue;
        }
        let Some(path) = normalize_mtree_path(&entry.path.to_string_lossy()) else {
            continue;
        };
        if is_package_metadata_path(&path) {
            continue;
        }
        if let Some(hash) = &entry.sha256 {
            map.insert(path, hash.clone());
        }
    }
    map
}
