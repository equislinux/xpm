//! System upgrade planning and execution.

use super::*;

/// Local-database metadata recorded for every package installed in a run.
struct UpgradePlanItem {
    repo: Repository,
    entry: RepoEntry,
    local_version: String,
    is_new: bool,
    replaced: Option<(String, String, InstallReason)>,
}

pub(super) fn cmd_upgrade(
    config: &XpmConfig,
    args: &cli::UpgradeArgs,
    no_confirm: bool,
) -> Result<()> {
    println!(":: Starting full system upgrade...");
    if !args.ignore.is_empty() {
        println!("   ignoring: {}", args.ignore.join(", "));
    }

    // Always refresh sync databases first (equivalent to pacman -Syu behavior).
    cmd_sync(config, &cli::SyncArgs { force: false })?;

    let local_db_dir = config.options.db_path.join("local");
    let sync_dir = config.options.db_path.join("sync");
    let cache_dir = &config.options.cache_dir;
    let arch = config
        .options
        .architecture
        .clone()
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let assumed = assume_installed_candidates(&args.assume_installed)
        .context("invalid --assume-installed")?;

    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create cache dir {}", cache_dir.display()))?;

    let overwrite = conflict::OverwritePatterns::parse(&args.overwrite);
    let mut target_manifests: Vec<conflict::TargetManifest> = Vec::new();
    let mut target_pkgs: Vec<depcheck::InstallTarget> = Vec::new();
    let installed = read_installed_versions(&local_db_dir)?;
    if installed.is_empty() {
        println!(":: No packages are currently installed in xpm local database.");
        return Ok(());
    }

    let remote_latest = read_latest_remote_entries(config, &sync_dir)?;

    // Sysupgrade replacements: scan repositories in priority order before the
    // normal same-name upgrade pass.
    let all_sync_entries = read_all_sync_entries(config, &sync_dir)?;
    let repo_ranks: HashMap<String, usize> = config
        .repositories
        .iter()
        .enumerate()
        .map(|(rank, repo)| (repo.name.clone(), rank))
        .collect();
    let replacement_candidates: Vec<xpm_core::replace::ReplacementCandidate> = all_sync_entries
        .iter()
        .filter_map(|(repo, entry)| {
            let rank = *repo_ranks.get(&repo.name)?;
            Some(xpm_core::replace::ReplacementCandidate {
                name: entry.name.clone(),
                version: Version::parse(&entry.version),
                repo_rank: rank,
                replaces: entry
                    .replaces
                    .iter()
                    .map(|replace| DepConstraint::parse(replace))
                    .collect(),
                provides: entry
                    .provides
                    .iter()
                    .map(|provide| DepConstraint::parse(provide))
                    .collect(),
                conflicts: entry
                    .conflicts
                    .iter()
                    .map(|conflict| DepConstraint::parse(conflict))
                    .collect(),
            })
        })
        .collect();
    let mut installed_groups: HashMap<String, Vec<String>> = HashMap::new();
    let mut installed_replacements: Vec<xpm_core::replace::InstalledReplacement> =
        Vec::with_capacity(installed.len());
    for (name, version) in &installed {
        let groups = local_db::read_groups(&local_db_dir, name)?;
        installed_groups.insert(name.clone(), groups.clone());
        installed_replacements.push(xpm_core::replace::InstalledReplacement {
            name: name.clone(),
            version: Version::parse(version),
            reason: InstallReason::read_optional(&local_db_dir, name)
                .unwrap_or(InstallReason::Explicit),
            groups,
        });
    }
    let mut ignore_patterns = args.ignore.clone();
    ignore_patterns.extend(config.options.ignore_pkg.iter().cloned());
    let mut ignore_groups = args.ignore_groups.clone();
    ignore_groups.extend(config.options.ignore_group.iter().cloned());
    let found_replacements = xpm_core::replace::find_replacements(
        &installed_replacements,
        &replacement_candidates,
        &ignore_patterns,
        &ignore_groups,
    );

    let mut replacement_new: HashMap<String, xpm_core::replace::Replacement> = HashMap::new();
    let mut replacement_old_names: HashSet<String> = HashSet::new();
    let mut replacement_entries: HashMap<String, (Repository, RepoEntry)> = HashMap::new();
    for replacement in found_replacements {
        let repo_name = config
            .repositories
            .get(replacement.repo_rank)
            .map(|repo| repo.name.clone())
            .unwrap_or_default();
        if !confirm_replace(
            &replacement.old_name,
            &repo_name,
            &replacement.new_name,
            no_confirm || args.print,
        )? {
            continue;
        }

        let selected = all_sync_entries
            .iter()
            .find(|(repo, entry)| {
                repo.name == repo_name
                    && entry.name == replacement.new_name
                    && Version::parse(&entry.version) == replacement.new_version
            })
            .cloned();
        let Some((repo, entry)) = selected else {
            continue;
        };

        replacement_old_names.insert(replacement.old_name.clone());
        replacement_entries.insert(replacement.new_name.clone(), (repo, entry));
        replacement_new.insert(replacement.new_name.clone(), replacement);
    }

    let mut planned = Vec::new();
    for (pkg_name, local_version) in &installed {
        if args.ignore.iter().any(|i| i == pkg_name)
            || config.options.ignore_pkg.iter().any(|i| i == pkg_name)
            || replacement_old_names.contains(pkg_name)
        {
            continue;
        }

        let Some((repo, entry)) = remote_latest.get(pkg_name) else {
            continue;
        };

        let local_groups = installed_groups
            .get(pkg_name)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if matches_ignore_group(local_groups, &ignore_groups)
            || matches_ignore_group(&entry.groups, &ignore_groups)
        {
            continue;
        }

        let ordering = Version::cmp_versions(&entry.version, local_version);
        if ordering.is_gt() || (args.force && ordering.is_eq()) {
            planned.push((repo.clone(), entry.clone(), local_version.clone()));
        }
    }

    if planned.is_empty() && replacement_new.is_empty() {
        println!(":: Nothing to do. System is up to date.");
        return Ok(());
    }

    // Include the transitive closure of the upgraded packages so new or
    // newly-required dependencies are installed in the same run.
    let mut planned_names: HashSet<String> = planned
        .iter()
        .map(|(_, entry, _)| entry.name.clone())
        .collect();
    planned_names.extend(replacement_new.keys().cloned());
    let mut candidates = Vec::new();
    // Walk the databases in repository priority order so provider ranking is
    // deterministic (HashMap iteration is not).
    for (repo, entry) in &all_sync_entries {
        let is_latest =
            remote_latest
                .get(&entry.name)
                .is_some_and(|(latest_repo, latest_entry)| {
                    latest_repo.name == repo.name && latest_entry.version == entry.version
                });
        if is_latest {
            candidates.push(resolver_candidate(&repo.name, entry));
        }
    }
    let requirements: Vec<Requirement> = planned_names
        .iter()
        .map(|name| Requirement {
            name: name.clone(),
            op: None,
            version: None,
            repo: None,
        })
        .collect();
    let closure: Vec<(PackageCandidate, bool)> = if assumed.is_empty() {
        resolve_closure(candidates, &requirements).with_context(|| {
            format!(
                "failed to resolve the upgrade closure for: {}",
                planned_names.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        })?
    } else {
        resolve_closure_with_installed(candidates, assumed.clone(), &requirements)
            .with_context(|| {
                format!(
                    "failed to resolve the upgrade closure for: {}",
                    planned_names.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            })?
            .into_iter()
            .filter(|(_, _, installed)| !installed)
            .map(|(candidate, explicit, _)| (candidate, explicit))
            .collect()
    };

    let mut plan: Vec<UpgradePlanItem> = Vec::new();
    let mut planned_replacements: HashSet<String> = HashSet::new();
    for (candidate, _explicit) in closure {
        if let Some(replacement) = replacement_new.get(&candidate.name) {
            if let Some((repo, entry)) = replacement_entries.get(&candidate.name) {
                plan.push(UpgradePlanItem {
                    repo: repo.clone(),
                    entry: entry.clone(),
                    local_version: String::new(),
                    is_new: true,
                    replaced: Some((
                        replacement.old_name.clone(),
                        replacement.old_version.to_string(),
                        replacement.old_reason,
                    )),
                });
                planned_replacements.insert(candidate.name.clone());
            }
            continue;
        }
        if replacement_old_names.contains(&candidate.name) {
            continue;
        }

        let Some((repo, entry)) = remote_latest.get(&candidate.name) else {
            continue;
        };
        if Version::parse(&entry.version) != candidate.version {
            continue;
        }
        match installed.get(&candidate.name) {
            Some(local) => {
                let ordering = Version::cmp_versions(&entry.version, local);
                let requested = planned_names.contains(&candidate.name);
                if ordering.is_gt() || (args.force && ordering.is_eq() && requested) {
                    plan.push(UpgradePlanItem {
                        repo: repo.clone(),
                        entry: entry.clone(),
                        local_version: local.clone(),
                        is_new: false,
                        replaced: None,
                    });
                }
            }
            None => plan.push(UpgradePlanItem {
                repo: repo.clone(),
                entry: entry.clone(),
                local_version: String::new(),
                is_new: true,
                replaced: None,
            }),
        }
    }

    for (name, replacement) in &replacement_new {
        if planned_replacements.contains(name) {
            continue;
        }
        let Some((repo, entry)) = replacement_entries.get(name) else {
            continue;
        };
        plan.push(UpgradePlanItem {
            repo: repo.clone(),
            entry: entry.clone(),
            local_version: String::new(),
            is_new: true,
            replaced: Some((
                replacement.old_name.clone(),
                replacement.old_version.to_string(),
                replacement.old_reason,
            )),
        });
    }

    let mut removed_upgrade_names: Vec<String> = plan
        .iter()
        .filter(|item| !item.is_new)
        .map(|item| item.entry.name.clone())
        .collect();
    removed_upgrade_names.extend(
        plan.iter()
            .filter_map(|item| item.replaced.as_ref().map(|(old, _, _)| old.clone())),
    );
    removed_upgrade_names.sort();
    removed_upgrade_names.dedup();

    let replacement_count = plan.iter().filter(|item| item.replaced.is_some()).count();
    let upgrade_count = plan
        .iter()
        .filter(|item| !item.is_new && item.replaced.is_none())
        .count();
    let new_dep_count = plan
        .iter()
        .filter(|item| item.is_new && item.replaced.is_none())
        .count();
    if replacement_count > 0 {
        println!(
            ":: Packages to upgrade: {upgrade_count} (+{new_dep_count} new dependency/ies, {replacement_count} replacement(s))"
        );
    } else if new_dep_count > 0 {
        println!(":: Packages to upgrade: {upgrade_count} (+{new_dep_count} new dependency/ies)");
    } else {
        println!(":: Packages to upgrade: {upgrade_count}");
    }
    for item in &plan {
        let entry = &item.entry;
        if let Some((old_name, old_version, _)) = &item.replaced {
            println!(
                "   {} {} -> {} {} (replaces {})",
                old_name, old_version, entry.name, entry.version, old_name
            );
        } else if item.is_new {
            println!("   {} {} (new dependency)", entry.name, entry.version);
        } else {
            println!(
                "   {} {} -> {}",
                entry.name, item.local_version, entry.version
            );
        }
    }

    if args.print {
        return Ok(());
    }

    confirm_action(":: Proceed with upgrade? [y/N] ", no_confirm)?;

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

    for item in plan {
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
    installed_pkgs.extend(assume_installed_pkgs(&assumed));
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

    depcheck::check_installed_closure(&installed_pkgs, &removed_upgrade_names, &added_pkgs)
        .context("upgrade reverse-dependency check failed")?;
    depcheck::check_install(&installed_pkgs, &target_pkgs)
        .context("upgrade dependency check failed")?;
    check_transaction_file_conflicts(
        config,
        &local_db_dir,
        &target_manifests,
        &removed_upgrade_names,
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
