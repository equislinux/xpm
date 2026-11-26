//! Search matching for repository entries.

use super::*;

/// Case-insensitive match of `query` against an entry's name, description or
/// provides (the `xpm search` contract).
pub fn matches_query(entry: &RepoEntry, query: &str) -> bool {
    let query = query.to_lowercase();
    if query.is_empty() {
        return false;
    }

    entry.name.to_lowercase().contains(&query)
        || entry
            .description
            .as_deref()
            .is_some_and(|desc| desc.to_lowercase().contains(&query))
        || entry
            .provides
            .iter()
            .any(|provide| provide.to_lowercase().contains(&query))
}
