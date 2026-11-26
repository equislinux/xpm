//! Repository database synchronization.

use super::*;

pub(super) fn cmd_sync(config: &XpmConfig, args: &cli::SyncArgs) -> Result<()> {
    let force = if args.force { " (forced)" } else { "" };
    println!(":: Synchronizing package databases{force}...");

    let _lock = FileLock::acquire(&config.options.db_path.join("transaction.lock"))?;
    let mut failures: Vec<(String, String)> = Vec::new();

    let arch = config
        .options
        .architecture
        .clone()
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let sync_dir = config.options.db_path.join("sync");

    let remote_results = sync_repositories_in_parallel(
        &config.repositories,
        &arch,
        &sync_dir,
        3,
        config.options.parallel_downloads.max(1) as usize,
        config
            .options
            .database_sig_level
            .unwrap_or(config.options.sig_level),
        config.options.keyring_path(),
    );
    let mut remote_by_repo: HashMap<String, Result<xpm_core::repo_sync::RepoSyncResult, XpmError>> =
        remote_results.into_iter().collect();

    for repo in &config.repositories {
        println!("   {} — {} server(s)", repo.name, repo.server.len());

        match remote_by_repo
            .remove(&repo.name)
            .unwrap_or_else(|| Err(XpmError::Other("missing remote sync result".to_string())))
        {
            Ok(result) => {
                println!("     mirror: {}", result.mirror);
                if result.db_downloaded {
                    println!("     remote: {}.db updated", repo.name);
                }
                if result.files_downloaded {
                    println!("     remote: {}.files updated", repo.name);
                }
            }
            Err(err) => {
                tracing::warn!(repo = %repo.name, error = %err, "remote sync failed");
                println!("     remote: unavailable ({err})");
                failures.push((repo.name.clone(), format!("{err}")));
            }
        }

        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if db_path.exists() {
            match parse_sync_db_cached(&db_path, &repo.name) {
                Ok(mut db) => {
                    let files_path = sync_dir.join(format!("{}.files", repo.name));
                    if files_path.exists() {
                        if let Err(err) = merge_files_db(&files_path, &mut db) {
                            tracing::warn!(
                                repo = %repo.name,
                                path = %files_path.display(),
                                error = %err,
                                "failed to parse .files database"
                            );
                        }
                    }

                    let with_files = db.entries.iter().filter(|e| !e.files.is_empty()).count();
                    println!(
                        "     local db: {} package(s) loaded ({} with file lists)",
                        db.entries.len(),
                        with_files
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        repo = %repo.name,
                        path = %db_path.display(),
                        error = %err,
                        "failed to parse local sync database"
                    );
                    println!("     local db: parse error ({err})");
                    failures.push((repo.name.clone(), format!("invalid local database: {err}")));
                }
            }
        } else {
            println!(
                "     local db: not found at {}",
                display_rel_or_abs(&db_path)
            );
        }
    }
    if !failures.is_empty() {
        let summary = failures
            .iter()
            .map(|(repo, error)| format!("{repo}: {error}"))
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("failed to synchronize all databases ({summary})");
    }

    println!(":: Sync complete.");
    Ok(())
}

pub(super) fn read_all_sync_entries(
    config: &XpmConfig,
    sync_dir: &Path,
) -> Result<Vec<(Repository, RepoEntry)>> {
    let mut entries = Vec::new();
    for repo in &config.repositories {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            continue;
        }
        let db = parse_sync_db_cached(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", display_rel_or_abs(&db_path)))?;
        for entry in db.entries {
            entries.push((repo.clone(), entry));
        }
    }
    Ok(entries)
}

pub(super) fn sync_repositories_in_parallel(
    repositories: &[Repository],
    arch: &str,
    sync_dir: &Path,
    retries: u32,
    max_parallel: usize,
    default_database_sig_level: xpm_core::config::SigLevel,
    keyring_path: PathBuf,
) -> Vec<(
    String,
    Result<xpm_core::repo_sync::RepoSyncResult, XpmError>,
)> {
    let mut results = Vec::with_capacity(repositories.len());

    for chunk in repositories.chunks(max_parallel.max(1)) {
        let mut handles = Vec::with_capacity(chunk.len());

        for repo in chunk {
            let repo_clone = repo.clone();
            let arch_owned = arch.to_string();
            let sync_dir_owned = sync_dir.to_path_buf();
            let keyring_path_owned = keyring_path.clone();

            handles.push(thread::spawn(move || {
                let name = repo_clone.name.clone();
                let result = sync_repo_databases(
                    &repo_clone,
                    &arch_owned,
                    &sync_dir_owned,
                    retries,
                    default_database_sig_level,
                    &keyring_path_owned,
                );
                (name, result)
            }));
        }

        for handle in handles {
            match handle.join() {
                Ok(result) => results.push(result),
                Err(_) => results.push((
                    "unknown".to_string(),
                    Err(XpmError::Other("sync worker thread panicked".to_string())),
                )),
            }
        }
    }

    results
}

pub(super) fn sync_repositories<'a>(
    config: &'a XpmConfig,
    requested: Option<&str>,
) -> Result<Vec<&'a Repository>> {
    if let Some(name) = requested {
        let selected: Vec<&Repository> = config
            .repositories
            .iter()
            .filter(|repo| repo.name == name)
            .collect();
        if selected.is_empty() {
            anyhow::bail!("repository '{name}' was not found");
        }
        Ok(selected)
    } else {
        Ok(config.repositories.iter().collect())
    }
}
