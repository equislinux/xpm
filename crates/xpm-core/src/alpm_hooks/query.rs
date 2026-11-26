//! Target glob matching and installed-package queries.

use super::*;

/// Wildcard match supporting `*` (any sequence) and `?` (one character),
/// which covers the target globs used by distribution hooks.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();

    let (mut p, mut t) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut mark = 0usize;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Convenience: set of `Depends` names present in a local database.
pub fn installed_names(local_db_dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Ok(entries) = fs::read_dir(local_db_dir) {
        for entry in entries.flatten() {
            if entry.path().join("version").is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    names.insert(name.to_string());
                }
            }
        }
    }
    names
}
