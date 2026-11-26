//! Path normalization and ordering helpers.

use super::*;

pub(super) fn normalize_list(name: &str, paths: &[String]) -> XpmResult<Vec<String>> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for raw in paths {
        if let Some(path) = normalize_path(name, raw)? {
            if seen.insert(path.clone()) {
                normalized.push(path);
            }
        }
    }

    Ok(normalized)
}

pub(super) fn normalize_path(name: &str, raw: &str) -> XpmResult<Option<String>> {
    if raw.is_empty() || raw == "/" {
        return Ok(None);
    }

    if raw.contains('\0') || raw.starts_with('/') {
        return Err(unsafe_path(name, raw));
    }

    let mut path = raw;
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }

    if path.is_empty() || path == "." {
        return Ok(None);
    }

    if path.split('/').any(|component| component == "..") {
        return Err(unsafe_path(name, raw));
    }

    Ok(Some(path.to_string()))
}

pub(super) fn unsafe_path(name: &str, path: &str) -> XpmError {
    XpmError::Package(format!("unsafe path in package '{name}': {path}"))
}

pub(super) fn rooted(root: &Path, relative: &str) -> String {
    root.join(relative).to_string_lossy().into_owned()
}

pub(super) fn path_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();

    let mut index = 0;
    while index < left.len() && index < right.len() && left[index] == right[index] {
        index += 1;
    }

    let mut left_index = index;
    let mut right_index = index;

    if left_index == left.len() && right_index < right.len() && right[right_index] == b'/' {
        right_index += 1;
    } else if right_index == right.len() && left_index < left.len() && left[left_index] == b'/' {
        left_index += 1;
    }

    left[left_index..].cmp(&right[right_index..])
}
