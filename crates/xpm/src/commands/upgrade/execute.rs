//! Upgrade execution: download, verify, transact and record.

use super::*;

pub(super) fn run(config: &XpmConfig, args: &cli::UpgradeArgs, plan: UpgradePlan) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let cache_dir = &config.options.cache_dir;
    let arch = config
        .options
        .architecture
        .clone()
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let overwrite = conflict::OverwritePatterns::parse(&args.overwrite);
    let mut target_manifests: Vec<conflict::TargetManifest> = Vec::new();
    let mut target_pkgs: Vec<depcheck::InstallTarget> = Vec::new();

    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(
        config.options.shell_integration && config.options.root_dir != Path::new("/"),
    );
    tx.set_scriptlet_mode(config.options.scriptlet_mode);

    let mut journal_pkgs = Vec::new();
    let mut installed_records: Vec<InstalledRecord> = Vec::new();

    for item in plan.items {
        let repo = &item.repo;
        let entry = &item.entry;
        let local_version = &item.local_version;
        let is_new = item.is_new;
        let filename = entry.resolved_filename(&arch);

        let dest = cache_dir.join(&filename);
        let urls = package_download_candidates(repo, &arch, entry);
        let mirror = download_first_available(&urls, &dest, 3).with_context(|| {
            format!(
                "failed to download '{}' from repo '{}'",
                entry.name, repo.name
            )
        })?;

        let sig_level = repo.sig_level.unwrap_or(config.options.sig_level);
        let keyring_path = config.options.keyring_path();
        let sig_url = format!("{mirror}.sig");
        verify_remote_signature(&dest, &sig_url, sig_level, &keyring_path, 3).with_context(
            || {
                format!(
                    "signature verification failed for '{}' from repo '{}'",
                    entry.name, repo.name
                )
            },
        )?;

        let metadata = Arc::new(
            read_metadata(&dest)
                .with_context(|| format!("failed to read metadata from {}", dest.display()))?,
        );
        tx.cache_package_metadata(dest.clone(), metadata.clone());
        target_manifests.push(target_manifest(&local_db_dir, &entry.name, &metadata)?);
        target_pkgs.push(install_target(&metadata.meta, &entry.version));

        // Replacements remove the old package first; the new package is a
        // fresh install that inherits the old reason.
        let mut journal_pkg = if let Some((old_name, old_version, _)) = &item.replaced {
            tx.add_remove(old_name.clone())
                .context("failed to add replacement removal to transaction")?;
            journal_pkgs.push(JournalPackage::remove(old_name, old_version));
            tx.add_install(entry.name.clone(), entry.version.clone(), dest.clone())
                .context("failed to add replacement install to transaction")?;
            JournalPackage::install(&entry.name, &entry.version)
        } else if is_new {
            tx.add_install(entry.name.clone(), entry.version.clone(), dest.clone())
                .context("failed to add install op to transaction")?;
            JournalPackage::install(&entry.name, &entry.version)
        } else {
            tx.add_upgrade(
                entry.name.clone(),
                local_version.clone(),
                entry.version.clone(),
                dest.clone(),
            )
            .context("failed to add upgrade op to transaction")?;
            JournalPackage::upgrade(&entry.name, local_version, &entry.version)
        };
        journal_pkg = journal_pkg.with_repo(&repo.name).with_source(&mirror);
        if let Some(sum) = entry.sha256sum.as_deref() {
            journal_pkg = journal_pkg.with_sha256(sum);
        }
        journal_pkgs.push(journal_pkg);

        let record_reason = if let Some((_, _, old_reason)) = &item.replaced {
            *old_reason
        } else if is_new {
            InstallReason::Dep
        } else {
            InstallReason::read_optional(&local_db_dir, &entry.name)
                .unwrap_or(InstallReason::Explicit)
        };

        installed_records.push(InstalledRecord {
            name: entry.name.clone(),
            repo: repo.name.clone(),
            depends: metadata.meta.depends.clone(),
            provides: metadata.meta.provides.clone(),
            groups: metadata.meta.groups.clone(),
            reason: record_reason,
        });
    }

    if args.download_only {
        println!(":: Download complete.");
        return Ok(());
    }

    let mut installed_pkgs = load_installed_pkgs(&local_db_dir)?;
    installed_pkgs.extend(assume_installed_pkgs(&plan.assumed));
    let added_pkgs: Vec<depcheck::InstalledPkg> = target_pkgs
        .iter()
        .map(|target| depcheck::InstalledPkg {
            name: target.name.clone(),
            version: target.version.clone(),
            depends: target.depends.clone(),
            provides: target.provides.clone(),
            reason: InstallReason::Explicit,
        })
        .collect();
    let transaction_paths: HashSet<String> = target_manifests
        .iter()
        .flat_map(|manifest| manifest.files.iter().cloned())
        .collect();
    tx.set_transaction_paths(Arc::new(transaction_paths));

    depcheck::check_installed_closure(&installed_pkgs, &plan.removed_upgrade_names, &added_pkgs)
        .context("upgrade reverse-dependency check failed")?;
    depcheck::check_install(&installed_pkgs, &target_pkgs)
        .context("upgrade dependency check failed")?;
    check_transaction_file_conflicts(
        config,
        &local_db_dir,
        &target_manifests,
        &plan.removed_upgrade_names,
        &overwrite,
    )?;

    commit_transaction(
        config,
        "upgrade",
        journal_pkgs,
        hook_paths_from_targets(&target_manifests),
        &mut tx,
    )?;

    for record in &installed_records {
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

    println!(":: Upgrade complete.");
    Ok(())
}
