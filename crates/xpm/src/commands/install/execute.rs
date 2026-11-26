//! Install execution: prefetch, verify, transact and record.

use super::*;

pub(super) fn run(
    config: &XpmConfig,
    args: &cli::InstallArgs,
    no_confirm: bool,
    plan: InstallPlan,
) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let cache_dir = &config.options.cache_dir;
    let arch = plan.arch.clone();

    // Create transaction
    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;

    // Setup hooks chain
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(
        config.options.shell_integration && config.options.root_dir != Path::new("/"),
    );
    tx.set_scriptlet_mode(config.options.scriptlet_mode);

    let mut journal_pkgs = Vec::new();
    let mut removed_names: Vec<String> = Vec::new();
    let mut installed: Vec<InstalledRecord> = Vec::new();
    let overwrite = conflict::OverwritePatterns::parse(&args.overwrite);
    let mut target_manifests: Vec<conflict::TargetManifest> = Vec::new();
    let mut target_pkgs: Vec<depcheck::InstallTarget> = Vec::new();
    let installed_versions = read_installed_versions(&local_db_dir)?;
    let mirrors = prefetch_downloads(
        &plan.resolved,
        &installed_versions,
        args.needed,
        &arch,
        cache_dir,
        config.options.parallel_downloads,
    )?;
    let keyring_path = config.options.keyring_path();
    let verification = verify_signatures_parallel(
        &plan.resolved,
        &mirrors,
        &installed_versions,
        args.needed,
        &arch,
        cache_dir,
        &keyring_path,
        config.options.sig_level,
        config.options.parallel_downloads,
    )?;

    // Phase 1a: local package files (already "downloaded")
    for (path, metadata) in &plan.local_metadata {
        let path = path.as_path();
        let pkg_name = metadata.meta.name.clone();
        if pkg_name.is_empty() {
            anyhow::bail!("package file {} has no pkgname", path.display());
        }
        let version = metadata.meta.full_version();

        if args.download_only {
            println!(
                "   skipped (--download-only): local file {}",
                path.display()
            );
            continue;
        }

        let installed_version = installed_versions.get(&pkg_name);
        if args.needed
            && installed_version
                .is_some_and(|current| Version::parse(current) == Version::parse(&version))
        {
            println!("warning: {pkg_name}-{version} is up to date -- skipping");
            continue;
        }

        let is_upgrade = installed_version.is_some();
        let pkg_reason = if args.as_deps {
            InstallReason::Dep
        } else if args.as_explicit {
            InstallReason::Explicit
        } else if is_upgrade {
            InstallReason::read_optional(&local_db_dir, &pkg_name)
                .unwrap_or(InstallReason::Explicit)
        } else {
            InstallReason::Explicit
        };

        if is_upgrade {
            println!(
                "   local: {} ({} {} -> {})",
                path.display(),
                pkg_name,
                installed_version.unwrap(),
                version
            );
        } else {
            println!("   local: {} ({} {})", path.display(), pkg_name, version);
        }
        tx.cache_package_metadata(path.to_path_buf(), metadata.clone());
        target_manifests.push(target_manifest(&local_db_dir, &pkg_name, metadata)?);
        target_pkgs.push(install_target(&metadata.meta, &version));

        if is_upgrade {
            let old_version = installed_version.unwrap().clone();
            removed_names.push(pkg_name.clone());
            journal_pkgs.push(JournalPackage::upgrade(&pkg_name, &old_version, &version));
            tx.add_upgrade(
                pkg_name.clone(),
                old_version,
                version.clone(),
                path.to_path_buf(),
            )
            .context("failed to add local upgrade to transaction")?;
        } else {
            journal_pkgs.push(JournalPackage::install(&pkg_name, &version));
            tx.add_install(pkg_name.clone(), version.clone(), path.to_path_buf())
                .context("failed to add local install to transaction")?;
        }
        installed.push(InstalledRecord {
            name: pkg_name.clone(),
            repo: "local".to_string(),
            depends: metadata.meta.depends.clone(),
            provides: metadata.meta.provides.clone(),
            groups: metadata.meta.groups.clone(),
            reason: pkg_reason,
        });
    }

    // Phase 1b: Download and validate the resolved plan (dependencies first)
    for (index, (repo, entry, explicit)) in plan.resolved.iter().enumerate() {
        let pkg_name = &entry.name;
        let installed_version = installed_versions.get(pkg_name);

        if args.needed
            && installed_version
                .is_some_and(|current| Version::parse(current) == Version::parse(&entry.version))
        {
            println!(
                "warning: {}-{} is up to date -- skipping",
                entry.name, entry.version
            );
            continue;
        }

        let filename = entry.resolved_filename(&arch);
        let dest = cache_dir.join(&filename);
        let mirror = mirrors[index].clone().ok_or_else(|| {
            anyhow::anyhow!(
                "failed to download '{}' from repo '{}'",
                pkg_name,
                repo.name
            )
        })?;

        if let Some(Some(Err(err))) = verification.get(index) {
            anyhow::bail!("{err}");
        }

        let metadata = Arc::new(
            read_metadata(&dest)
                .with_context(|| format!("failed to read metadata from {}", dest.display()))?,
        );
        tx.cache_package_metadata(dest.clone(), metadata.clone());
        target_manifests.push(target_manifest(&local_db_dir, &entry.name, &metadata)?);
        target_pkgs.push(install_target(&metadata.meta, &entry.version));

        let is_upgrade = installed_version.is_some();
        let pkg_reason = if args.as_deps {
            InstallReason::Dep
        } else if args.as_explicit {
            InstallReason::Explicit
        } else if is_upgrade {
            InstallReason::read_optional(&local_db_dir, &entry.name)
                .unwrap_or(InstallReason::Explicit)
        } else if *explicit {
            InstallReason::Explicit
        } else {
            InstallReason::Dep
        };

        if xpm_core::progress::verbose() {
            println!("   downloaded: {}", dest.display());
            println!("   source: {}", mirror);
        }

        let mut journal_pkg = if is_upgrade {
            JournalPackage::upgrade(&entry.name, installed_version.unwrap(), &entry.version)
        } else {
            JournalPackage::install(&entry.name, &entry.version)
        }
        .with_repo(&repo.name)
        .with_source(&mirror);
        if let Some(sum) = entry.sha256sum.as_deref() {
            journal_pkg = journal_pkg.with_sha256(sum);
        }
        journal_pkgs.push(journal_pkg);
        installed.push(InstalledRecord {
            name: entry.name.clone(),
            repo: repo.name.clone(),
            depends: metadata.meta.depends.clone(),
            provides: metadata.meta.provides.clone(),
            groups: metadata.meta.groups.clone(),
            reason: pkg_reason,
        });

        // Add to transaction
        if is_upgrade {
            removed_names.push(entry.name.clone());
            tx.add_upgrade(
                entry.name.clone(),
                installed_version.unwrap().clone(),
                entry.version.clone(),
                dest,
            )
            .context("failed to add upgrade to transaction")?;
        } else {
            tx.add_install(entry.name.clone(), entry.version.clone(), dest)
                .context("failed to add install to transaction")?;
        }
    }

    if journal_pkgs.is_empty() {
        if args.download_only {
            println!(":: Download complete.");
        } else {
            println!(":: Nothing to do.");
        }
        return Ok(());
    }

    if args.download_only {
        println!(":: Download complete.");
        return Ok(());
    }

    confirm_action(
        ":: Proceed with installation? [y/N] (download already completed) ",
        no_confirm,
    )?;

    let transaction_paths: HashSet<String> = target_manifests
        .iter()
        .flat_map(|manifest| manifest.files.iter().cloned())
        .collect();
    tx.set_transaction_paths(Arc::new(transaction_paths));

    let mut installed_pkgs = load_installed_pkgs(&local_db_dir)?;
    installed_pkgs.extend(assume_installed_pkgs(&plan.assumed));
    depcheck::check_install(&installed_pkgs, &target_pkgs)
        .context("installation dependency check failed")?;
    check_transaction_file_conflicts(
        config,
        &local_db_dir,
        &target_manifests,
        &removed_names,
        &overwrite,
    )?;

    // Phase 2/3: prepare, commit, hooks and journal.
    commit_transaction(
        config,
        "install",
        journal_pkgs,
        hook_paths_from_targets(&target_manifests),
        &mut tx,
    )?;

    for record in &installed {
        record
            .reason
            .write(&local_db_dir, &record.name)
            .with_context(|| format!("failed to record install reason for '{}'", record.name))?;
        local_db::write_origin(&local_db_dir, &record.name, &record.repo)
            .with_context(|| format!("failed to record origin for '{}'", record.name))?;
        local_db::write_depends(&local_db_dir, &record.name, &record.depends)
            .with_context(|| format!("failed to record dependencies for '{}'", record.name))?;
        local_db::write_provides(&local_db_dir, &record.name, &record.provides)
            .with_context(|| format!("failed to record provides for '{}'", record.name))?;
        local_db::write_groups(&local_db_dir, &record.name, &record.groups)
            .with_context(|| format!("failed to record groups for '{}'", record.name))?;
    }

    println!(":: {} package(s) installed successfully.", installed.len());
    if config.options.shell_integration && config.options.root_dir != Path::new("/") {
        println!(":: Shell integration enabled via ~/.local/bin shims.");
        println!(":: If this shell does not find new commands yet, run: hash -r");
        println!(":: For immediate PATH refresh, run: source ~/.zshrc or source ~/.bashrc");
    }
    Ok(())
}
