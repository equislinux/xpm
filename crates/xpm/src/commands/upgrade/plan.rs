//! Upgrade plan construction.

use super::*;

pub(super) fn build(
    config: &XpmConfig,
    args: &cli::UpgradeArgs,
    no_confirm: bool,
) -> Result<Option<UpgradePlan>> {
    let local_db_dir = config.options.db_path.join("local");
    let sync_dir = config.options.db_path.join("sync");
    let assumed = assume_installed_candidates(&args.assume_installed)
        .context("invalid --assume-installed")?;

    std::fs::create_dir_all(&config.options.cache_dir).with_context(|| {
        format!(
            "failed to create cache dir {}",
            config.options.cache_dir.display()
        )
    })?;

    let installed = read_installed_versions(&local_db_dir)?;
    if installed.is_empty() {
        println!(":: No packages are currently installed in xpm local database.");
        return Ok(None);
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
        return Ok(None);
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

    Ok(Some(UpgradePlan {
        items: plan,
        removed_upgrade_names,
        assumed,
    }))
}
