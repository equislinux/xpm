//! Installed/packaged `.MTREE` hash maps.

use super::*;

/// Builds the `path -> sha256` map from raw `.MTREE` bytes.
pub(super) fn mtree_hashes(mtree: Option<&[u8]>) -> HashMap<String, String> {
    match mtree {
        Some(bytes) => crate::package::mtree::parse_mtree(bytes)
            .map(|entries| crate::local_db::mtree_hash_map(&entries))
            .unwrap_or_default(),
        None => HashMap::new(),
    }
}

/// Loads the installed package's stored `.MTREE` hash map, if any.
pub(crate) fn load_installed_hashes(context: &HookContext) -> XpmResult<HashMap<String, String>> {
    let Some(raw) = crate::local_db::read_mtree(&context.local_db_dir, &context.pkg_name)? else {
        return Ok(HashMap::new());
    };
    Ok(mtree_hashes(Some(raw.as_slice())))
}
