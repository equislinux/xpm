//! Repository database synchronization.

use super::*;

pub fn sync_repo_databases(
    repo: &Repository,
    arch: &str,
    sync_dir: &Path,
    retries: u32,
    default_database_sig_level: SigLevel,
    keyring_path: &Path,
) -> XpmResult<RepoSyncResult> {
    fs::create_dir_all(sync_dir)?;

    let db_dest = sync_dir.join(format!("{}.db", repo.name));
    let files_dest = sync_dir.join(format!("{}.files", repo.name));
    let db_temp = sync_dir.join(format!(".{}.db.part", repo.name));
    let files_temp = sync_dir.join(format!(".{}.files.part", repo.name));
    let sig_level = repo
        .database_sig_level
        .unwrap_or(default_database_sig_level);

    let mut last_error: Option<XpmError> = None;

    for server in &repo.server {
        let base = expand_repo_url(server, &repo.name, arch);
        let base = base.trim_end_matches('/');
        let db_url = format!("{base}/{}.db", repo.name);
        let db_sig_url = format!("{db_url}.sig");

        match download_with_retries(
            &db_url,
            &db_temp,
            retries,
            true,
            None,
            None,
            Some(METADATA_TIMEOUT),
            true,
        ) {
            Ok(()) => {
                if let Err(err) =
                    verify_remote_signature(&db_temp, &db_sig_url, sig_level, keyring_path, retries)
                {
                    let _ = fs::remove_file(&db_temp);
                    last_error = Some(err);
                    continue;
                }

                let files_url = format!("{base}/{}.files", repo.name);
                let files_downloaded = match download_with_retries(
                    &files_url,
                    &files_temp,
                    retries,
                    true,
                    None,
                    None,
                    Some(METADATA_TIMEOUT),
                    true,
                ) {
                    Ok(()) => {
                        let files_sig_url = format!("{files_url}.sig");
                        match verify_remote_signature(
                            &files_temp,
                            &files_sig_url,
                            sig_level,
                            keyring_path,
                            retries,
                        ) {
                            Ok(()) => {
                                fs::rename(&files_temp, &files_dest)?;
                                true
                            }
                            Err(err) => {
                                let _ = fs::remove_file(&files_temp);
                                tracing::warn!(
                                    repo = %repo.name,
                                    error = %err,
                                    "optional .files signature failed; keeping previous file list"
                                );
                                false
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(repo = %repo.name, error = %err, "failed to download .files database");
                        false
                    }
                };

                fs::rename(&db_temp, &db_dest)?;
                return Ok(RepoSyncResult {
                    mirror: base.to_string(),
                    db_downloaded: true,
                    files_downloaded,
                });
            }
            Err(err) => {
                let _ = fs::remove_file(&db_temp);
                last_error = Some(err);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        XpmError::Database(format!(
            "no usable mirror found for repository '{}'",
            repo.name
        ))
    }))
}
