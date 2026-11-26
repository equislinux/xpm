//! Sync database parsing and cache.

use super::*;

/// Parse a repository sync database archive (`<repo>.db`).
pub fn parse_sync_db(path: &Path, repo_name: &str) -> XpmResult<SyncDb> {
    let bytes = fs::read(path)?;
    parse_sync_db_bytes(&bytes, repo_name)
}

#[derive(Serialize, Deserialize)]
pub(super) struct SyncCache {
    modified_ns: u128,
    len: u64,
    repo: String,
    db: SyncDb,
}

/// Parse a sync database through a persistent JSON cache keyed by mtime/size.
/// The first run pays the tar/gzip parse; subsequent commands load JSON.
pub fn parse_sync_db_cached(path: &Path, repo_name: &str) -> XpmResult<SyncDb> {
    let metadata = fs::metadata(path)?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let cache_path = path.with_extension("cache.json");
    if let Ok(text) = fs::read_to_string(&cache_path) {
        if let Ok(cache) = serde_json::from_str::<SyncCache>(&text) {
            if cache.modified_ns == modified_ns
                && cache.len == metadata.len()
                && cache.repo == repo_name
            {
                return Ok(cache.db);
            }
        }
    }

    let db = parse_sync_db(path, repo_name)?;
    let cache = SyncCache {
        modified_ns,
        len: metadata.len(),
        repo: repo_name.to_string(),
        db: db.clone(),
    };
    if let Ok(text) = serde_json::to_string(&cache) {
        let tmp = cache_path.with_extension("tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &cache_path);
        }
    }
    Ok(db)
}

pub(super) fn parse_sync_db_bytes(bytes: &[u8], repo_name: &str) -> XpmResult<SyncDb> {
    let mut packages = read_repo_archive(bytes)?;
    let mut entries = Vec::new();

    for (pkg_id, pkg) in packages.drain() {
        let mut merged = HashMap::<String, Vec<String>>::new();

        if let Some(desc) = pkg.desc.as_deref() {
            merge_sections(&mut merged, parse_alpm_sections(desc));
        }
        if let Some(depends) = pkg.depends.as_deref() {
            merge_sections(&mut merged, parse_alpm_sections(depends));
        }

        let entry = repo_entry_from_sections(&pkg_id, &merged)?;
        entries.push(entry);
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.version.cmp(&b.version)));

    Ok(SyncDb {
        repo: repo_name.to_string(),
        entries,
    })
}
