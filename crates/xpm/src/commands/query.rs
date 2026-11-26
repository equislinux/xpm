//! Local/sync queries, search, info and file listings.

use super::*;
use xpm_core::query;

pub(super) fn cmd_query(config: &XpmConfig, args: &cli::QueryArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");

    if let Some(path) = &args.file {
        if args.filter.is_some() {
            anyhow::bail!("xpm query: --file takes one package file and no name filter");
        }
        if args.changelog
            || args.native
            || args.owner.is_some()
            || args.explicit
            || args.deps
            || args.foreign
            || args.unrequired
            || args.orphans
            || args.upgrades
        {
            anyhow::bail!("xpm query: --file cannot be combined with database filters");
        }

        let metadata = read_metadata(path)
            .with_context(|| format!("failed to read package metadata from {}", path.display()))?;
        let name = &metadata.meta.name;
        let version = metadata.meta.full_version();
        if args.quiet {
            println!("{name}");
        } else {
            match args.format.as_str() {
                "tsv" => println!("{name}\t{version}"),
                _ => println!("{name} {version}"),
            }
        }
        return Ok(());
    }

    // `-Qc` reads the pacman-style changelog stored in the local database.
    // xpm installs do not extract a package's optional changelog yet, so this
    // currently covers entries imported from pacman (or populated externally).
    if args.changelog {
        let Some(package) = args.filter.as_deref() else {
            anyhow::bail!("xpm query: --changelog requires a package name");
        };
        if args.native
            || args.owner.is_some()
            || args.explicit
            || args.deps
            || args.foreign
            || args.unrequired
            || args.orphans
            || args.upgrades
            || args.search
        {
            anyhow::bail!("xpm query: --changelog cannot be combined with filters");
        }

        let Some(changelog) = local_db::read_changelog(&local_db_dir, package)? else {
            eprintln!("error: no changelog available for '{package}'.");
            exit_with(1);
        };
        if args.quiet {
            return Ok(());
        }
        print!("{changelog}");
        if !changelog.ends_with('\n') {
            println!();
        }
        return Ok(());
    }

    let local = query::load_local_packages(&local_db_dir)?;

    if let Some(owner_path) = args.owner.as_deref() {
        if args.filter.is_some() {
            anyhow::bail!(
                "xpm query: --owner takes one path and cannot be combined with a name filter"
            );
        }

        let owners = query::find_owners(&local, &config.options.root_dir, owner_path);
        if owners.is_empty() {
            eprintln!("error: no package owns {owner_path}");
            exit_with(1);
        }
        for owner in owners {
            if args.quiet {
                println!("{}", owner.name);
            } else {
                println!("{owner_path} is owned by {} {}", owner.name, owner.version);
            }
        }
        return Ok(());
    }

    if args.explicit && args.deps {
        anyhow::bail!("xpm query: --explicit and --deps are mutually exclusive");
    }

    if let Some(filter) = args.filter.as_deref() {
        if !args.search {
            let installed = local.iter().any(|package| package.name == filter);
            if !installed {
                eprintln!("error: package '{filter}' was not found");
                exit_with(1);
            }
        }
    }

    let mut packages: Vec<(String, String)> = local
        .iter()
        .map(|package| (package.name.clone(), package.version.clone()))
        .collect();

    if args.foreign {
        let sync_dir = config.options.db_path.join("sync");
        let sync_names = read_sync_package_names(config, &sync_dir)?;
        let foreign: HashSet<String> = query::find_foreign(&local, &sync_names)
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        packages.retain(|(name, _)| foreign.contains(name));
    }

    if args.native {
        let sync_dir = config.options.db_path.join("sync");
        let sync_names = read_sync_package_names(config, &sync_dir)?;
        packages.retain(|(name, _)| sync_names.contains(name));
    }

    if args.unrequired {
        let unrequired: HashSet<String> = query::find_unrequired(&local, args.deps)
            .into_iter()
            .collect();
        packages.retain(|(name, _)| unrequired.contains(name));
    }

    if args.upgrades {
        let sync_dir = config.options.db_path.join("sync");
        let remote = read_latest_remote_entries(config, &sync_dir)?;
        packages.retain(|(name, version)| {
            remote
                .get(name)
                .map(|(_, entry)| Version::cmp_versions(&entry.version, version).is_gt())
                .unwrap_or(false)
        });
    }

    if args.orphans {
        let mut installed_packages = Vec::with_capacity(packages.len());
        for (name, _) in &packages {
            let depends = local_db::read_depends(&local_db_dir, name)?;
            installed_packages.push(InstalledPackage {
                name: name.clone(),
                explicit: InstallReason::read(&local_db_dir, name) == InstallReason::Explicit,
                has_depends_record: depends.is_some(),
                depends: depends.unwrap_or_default(),
                provides: local_db::read_provides(&local_db_dir, name)?,
            });
        }
        let orphans: HashSet<String> = find_orphans(&installed_packages).into_iter().collect();
        packages.retain(|(name, _)| orphans.contains(name));
    }

    if args.explicit || args.deps {
        let wanted = if args.explicit {
            InstallReason::Explicit
        } else {
            InstallReason::Dep
        };
        retain_by_reason(&local_db_dir, &mut packages, wanted);
    }

    if let Some(filter) = args.filter.as_deref() {
        if args.search {
            packages.retain(|(name, _)| name.contains(filter));
        } else {
            packages.retain(|(name, _)| name == filter);
        }
    }

    packages.sort();
    for (name, version) in &packages {
        if args.quiet {
            println!("{name}");
        } else {
            match args.format.as_str() {
                "tsv" => println!("{name}\t{version}"),
                _ => println!("{name} {version}"),
            }
        }
    }
    Ok(())
}

pub(super) fn read_sync_package_names(
    config: &XpmConfig,
    sync_dir: &Path,
) -> Result<HashSet<String>> {
    let mut names = HashSet::new();

    for repo in &config.repositories {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            continue;
        }
        let db = parse_sync_db_cached(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", db_path.display()))?;
        names.extend(db.entries.into_iter().map(|entry| entry.name));
    }

    Ok(names)
}

pub(super) fn cmd_search(config: &XpmConfig, args: &cli::SearchArgs) -> Result<()> {
    use xpm_core::repo_db::{matches_query, RepoEntry};

    let mut hits: Vec<(String, RepoEntry)> = Vec::new();

    if args.local {
        let local_db_dir = config.options.db_path.join("local");
        for (name, version) in read_installed_versions(&local_db_dir)? {
            if name.to_lowercase().contains(&args.query.to_lowercase()) {
                hits.push((
                    "local".to_string(),
                    RepoEntry {
                        name,
                        version,
                        ..Default::default()
                    },
                ));
            }
        }
    } else {
        let sync_dir = config.options.db_path.join("sync");
        for repo in &config.repositories {
            let db_path = sync_dir.join(format!("{}.db", repo.name));
            if !db_path.exists() {
                continue;
            }
            let db = parse_sync_db_cached(&db_path, &repo.name)
                .with_context(|| format!("failed to parse sync db {}", db_path.display()))?;
            for entry in db.entries {
                if matches_query(&entry, &args.query) {
                    hits.push((repo.name.clone(), entry));
                }
            }
        }
    }

    hits.sort_by(|a, b| a.1.name.cmp(&b.1.name).then_with(|| a.0.cmp(&b.0)));

    if hits.is_empty() {
        println!(":: No packages found for '{}'.", args.query);
        return Ok(());
    }

    for (repo, entry) in &hits {
        println!("{}/{} {}", repo, entry.name, entry.version);
        if let Some(description) = &entry.description {
            println!("    {description}");
        }
    }
    Ok(())
}

pub(super) fn cmd_info(config: &XpmConfig, args: &cli::InfoArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let installed_version = local_db::read_version(&local_db_dir, &args.package);

    // Repository metadata (latest version, highest-priority repo wins), unless
    // the user explicitly asked for the local database only.
    let remote = if args.local {
        None
    } else {
        let sync_dir = config.options.db_path.join("sync");
        read_latest_remote_entries(config, &sync_dir)?.remove(&args.package)
    };

    if installed_version.is_none() && remote.is_none() {
        return Err(XpmError::PackageNotFound {
            name: args.package.clone(),
        }
        .into());
    }

    println!("Name            : {}", args.package);

    if let Some(version) = &installed_version {
        println!("Version         : {version}");
        println!(
            "Install Reason  : {}",
            InstallReason::read(&local_db_dir, &args.package).as_str()
        );
        println!(
            "Origin          : {}",
            local_db::read_origin(&local_db_dir, &args.package)
                .unwrap_or_else(|| "unknown".to_string())
        );
    }

    if let Some((repo, entry)) = &remote {
        println!("Repository      : {}", repo.name);
        if installed_version.is_none() {
            println!("Version         : {}", entry.version);
        }
        if let Some(description) = entry.description.as_deref() {
            println!("Description     : {description}");
        }
        if !entry.depends.is_empty() {
            println!("Depends On      : {}", entry.depends.join("  "));
        }
    }

    Ok(())
}

pub(super) fn cmd_files(config: &XpmConfig, args: &cli::FilesArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    if !local_db_dir.join(&args.package).is_dir() {
        return Err(XpmError::PackageNotFound {
            name: args.package.clone(),
        }
        .into());
    }

    let files = local_db::read_files(&local_db_dir, &args.package)
        .with_context(|| format!("failed to read the file list for '{}'", args.package))?;
    for file in &files {
        println!("{file}");
    }
    Ok(())
}
