//! Sync files database queries.

use super::*;

pub(super) fn normalize_files_query(raw: &str) -> String {
    let mut value = raw.trim();
    while let Some(stripped) = value.strip_prefix("./") {
        value = stripped;
    }
    value
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}

pub(super) fn cmd_file_db(config: &XpmConfig, args: &cli::FilesDbArgs) -> Result<()> {
    if args.refresh > 0 {
        cmd_sync(
            config,
            &cli::SyncArgs {
                force: args.refresh > 1,
            },
        )?;
    }

    if args.targets.is_empty() {
        if args.refresh > 0 {
            return Ok(());
        }
        anyhow::bail!("xpm file-db: at least one file or package target is required");
    }

    let sync_dir = config.options.db_path.join("sync");
    let mut entries: Vec<(String, RepoEntry)> = Vec::new();
    for repo in sync_repositories(config, args.repo.as_deref())? {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            if args.repo.is_some() {
                anyhow::bail!("database for repository '{}' was not found", repo.name);
            }
            continue;
        }
        let mut db = parse_sync_db_cached(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", display_rel_or_abs(&db_path)))?;
        let files_path = sync_dir.join(format!("{}.files", repo.name));
        if files_path.exists() {
            merge_files_db(&files_path, &mut db).with_context(|| {
                format!(
                    "failed to parse files db {}",
                    display_rel_or_abs(&files_path)
                )
            })?;
        }
        for entry in db.entries {
            entries.push((repo.name.clone(), entry));
        }
    }

    if args.list {
        let mut found = false;
        for target in &args.targets {
            for (_, entry) in &entries {
                if entry.name != *target {
                    continue;
                }
                found = true;
                if entry.files.is_empty() {
                    eprintln!("warning: no files database entry for '{}'", entry.name);
                    continue;
                }
                for file in &entry.files {
                    println!("{} {}", entry.name, file);
                }
            }
        }
        if !found {
            eprintln!("error: package '{}' was not found", args.targets.join(", "));
            exit_with(1);
        }
        return Ok(());
    }

    let mut found = false;
    for target in &args.targets {
        let normalized = normalize_files_query(target);
        for (repo_name, entry) in &entries {
            let owns = entry
                .files
                .iter()
                .any(|file| normalize_files_query(file) == normalized);
            if !owns {
                continue;
            }
            found = true;
            if args.quiet {
                println!("{repo_name}/{}", entry.name);
            } else {
                println!(
                    "{target} is owned by {}/{} {}",
                    repo_name, entry.name, entry.version
                );
            }
        }
    }

    if !found {
        eprintln!("error: no package owns {}", args.targets.join(", "));
        exit_with(1);
    }
    Ok(())
}
