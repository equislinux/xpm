//! Repository, group, listing, cache and pacman import commands.

use super::*;

pub(super) fn cmd_repo(config: &XpmConfig, args: &cli::RepoArgs) -> Result<()> {
    let manager = RepoManager::default_dir();

    match &args.action {
        cli::RepoAction::Add(add) => {
            manager
                .add(&add.name, &add.url)
                .with_context(|| format!("failed to add repository '{}'", add.name))?;
            println!(":: Repository '{}' added successfully.", add.name);
            println!("   url: {}", add.url);
            println!("   Run 'xpm sync' to refresh databases.");
        }
        cli::RepoAction::Remove(rm) => {
            manager
                .remove(&rm.name)
                .with_context(|| format!("failed to remove repository '{}'", rm.name))?;
            println!(":: Repository '{}' removed.", rm.name);
        }
        cli::RepoAction::List => {
            println!(":: Active repositories:");
            println!();

            // Predefined repos from config
            println!("   [predefined]");
            for repo in &config.repositories {
                let sig = repo.sig_level.unwrap_or(config.options.sig_level);
                println!(
                    "   {} ({} server(s), sig: {})",
                    repo.name,
                    repo.server.len(),
                    sig
                );
            }

            // User-added repos
            let user_repos = manager.list().context("failed to list user repositories")?;
            if !user_repos.is_empty() {
                println!();
                println!("   [user-added]");
                for repo in &user_repos {
                    println!("   {} — {}", repo.name, repo.server.join(", "));
                }
            }

            println!();
            let total = config.repositories.len() + user_repos.len();
            println!("   Total: {} repository(ies)", total);
        }
    }

    Ok(())
}

pub(super) fn cmd_groups(config: &XpmConfig, args: &cli::GroupsArgs) -> Result<()> {
    let mut index = groups::GroupIndex::new();

    if args.local {
        let local_db_dir = config.options.db_path.join("local");
        for (name, version) in read_installed_versions(&local_db_dir)? {
            index.add(groups::GroupPackage {
                name: name.clone(),
                version: Version::parse(&version),
                repo: None,
                groups: local_db::read_groups(&local_db_dir, &name)?,
            });
        }
    } else {
        let sync_dir = config.options.db_path.join("sync");
        for repo in &config.repositories {
            let db_path = sync_dir.join(format!("{}.db", repo.name));
            if !db_path.exists() {
                continue;
            }
            let db = parse_sync_db_cached(&db_path, &repo.name).with_context(|| {
                format!("failed to parse sync db {}", display_rel_or_abs(&db_path))
            })?;
            for entry in db.entries {
                index.add(groups::GroupPackage {
                    name: entry.name.clone(),
                    version: Version::parse(&entry.version),
                    repo: Some(repo.name.clone()),
                    groups: entry.groups,
                });
            }
        }
    }

    if args.names && args.group.is_none() {
        for group in index.group_names() {
            println!("{group}");
        }
        return Ok(());
    }

    let pairs = if let Some(group) = &args.group {
        let members = index.group_members_pairs(group);
        if members.is_empty() {
            if args.local {
                anyhow::bail!("group '{group}' was not found");
            }
            anyhow::bail!("package group '{group}' was not found");
        }
        members
    } else {
        index.group_pairs()
    };

    for (group, package) in pairs {
        if args.quiet {
            println!("{package}");
        } else {
            println!("{group} {package}");
        }
    }
    Ok(())
}

pub(super) fn cmd_list(config: &XpmConfig, args: &cli::ListArgs) -> Result<()> {
    let sync_dir = config.options.db_path.join("sync");
    let local_db_dir = config.options.db_path.join("local");
    let installed = read_installed_versions(&local_db_dir)?;

    for repo in sync_repositories(config, args.repo.as_deref())? {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            if args.repo.is_some() {
                anyhow::bail!("database for repository '{}' was not found", repo.name);
            }
            continue;
        }
        let db = parse_sync_db_cached(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", display_rel_or_abs(&db_path)))?;

        for entry in db.entries {
            if args.quiet {
                println!("{}", entry.name);
                continue;
            }
            let installed_note = match installed.get(&entry.name) {
                Some(version) if version == &entry.version => " [installed]".to_string(),
                Some(version) => format!(" [installed: {version}]"),
                None => String::new(),
            };
            println!(
                "{} {} {}{}",
                repo.name, entry.name, entry.version, installed_note
            );
        }
    }

    Ok(())
}

pub(super) fn cmd_clean(config: &XpmConfig, args: &cli::CleanArgs, no_confirm: bool) -> Result<()> {
    let cache_dir = &config.options.cache_dir;
    let local_db_dir = config.options.db_path.join("local");
    let installed = read_installed_versions(&local_db_dir)?;

    let prompt = if args.all {
        ":: Remove all cached package files? [y/N] "
    } else {
        ":: Remove cached package files that are not installed? [y/N] "
    };
    confirm_action(prompt, no_confirm)?;

    let report = clean_cache(cache_dir, &installed, args.all)
        .with_context(|| format!("failed to clean cache {}", cache_dir.display()))?;

    if report.removed.is_empty() {
        println!(":: No cached package files to remove.");
    } else {
        println!(
            ":: Removed {} cached package file(s) ({} bytes).",
            report.removed.len(),
            report.bytes_freed
        );
    }
    Ok(())
}

pub(super) fn cmd_import_pacman(config: &XpmConfig, args: &cli::ImportArgs) -> Result<()> {
    let dest = config.options.db_path.join("local");
    println!(
        ":: Importing pacman local database {} -> {}",
        args.from.display(),
        dest.display()
    );
    let report = xpm_core::migrate::import_pacman_local(&args.from, &dest, args.force)
        .context("failed to import pacman local database")?;

    println!(
        ":: Imported {} package(s), skipped {} already present.",
        report.imported, report.skipped
    );
    for (package, error) in &report.failed {
        eprintln!("   {package}: {error}");
    }
    if !report.failed.is_empty() {
        anyhow::bail!("{} package(s) failed to import", report.failed.len());
    }
    println!(":: Import complete. Run `xpm query` to inspect the local database.");
    Ok(())
}
