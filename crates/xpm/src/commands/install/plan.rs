//! Install planning: local files, dependency resolution and --print.

use super::*;

pub(super) fn build(config: &XpmConfig, args: &cli::InstallArgs) -> Result<Option<InstallPlan>> {
    if args.as_deps && args.as_explicit {
        anyhow::bail!("xpm install: --as-deps and --as-explicit are mutually exclusive");
    }

    let (local_files, package_names): (Vec<&String>, Vec<&String>) = args
        .packages
        .iter()
        .partition(|spec| Path::new(spec.as_str()).is_file());

    // Local packages still need their dependencies resolved from the sync DB.
    // This is what lets a preinstalled AUR package pull its official deps.
    let mut local_names = HashSet::new();
    let mut local_deps = Vec::new();
    let mut local_metadata = Vec::new();
    for spec in &local_files {
        let path = Path::new(spec.as_str());
        let metadata = Arc::new(
            read_metadata(path)
                .with_context(|| format!("failed to read metadata from {}", path.display()))?,
        );
        local_names.insert(metadata.meta.name.clone());
        local_deps.extend(metadata.meta.depends.iter().cloned());
        local_metadata.push((path.to_path_buf(), metadata));
    }
    let mut dependency_specs: Vec<String> =
        package_names.iter().map(|name| (*name).clone()).collect();
    for dep in local_deps {
        let dep_name = DepConstraint::parse(&dep).name;
        if !local_names.contains(&dep_name) && !dependency_specs.contains(&dep) {
            dependency_specs.push(dep);
        }
    }

    println!(
        ":: Resolving dependencies for: {}",
        args.packages.join(", ")
    );

    let arch = config
        .options
        .architecture
        .clone()
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let sync_dir = config.options.db_path.join("sync");
    let cache_dir = &config.options.cache_dir;
    let local_db_dir = config.options.db_path.join("local");
    let assumed = assume_installed_candidates(&args.assume_installed)
        .context("invalid --assume-installed")?;
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create cache dir {}", cache_dir.display()))?;

    // ── Resolve repository packages (if any) ────────────────────────
    let mut resolved: Vec<(Repository, RepoEntry, bool)> = Vec::new();
    if !dependency_specs.is_empty() {
        let mut candidates = Vec::new();
        let mut sync_entries: Vec<(Repository, RepoEntry)> = Vec::new();
        for repo in &config.repositories {
            let db_path = sync_dir.join(format!("{}.db", repo.name));
            if !db_path.exists() {
                continue;
            }
            let db = parse_sync_db_cached(&db_path, &repo.name).with_context(|| {
                format!("failed to parse sync db {}", display_rel_or_abs(&db_path))
            })?;
            for entry in db.entries {
                candidates.push(resolver_candidate(&repo.name, &entry));
                sync_entries.push((repo.clone(), entry));
            }
        }

        let mut group_index = groups::GroupIndex::new();
        for (repo, entry) in &sync_entries {
            group_index.add(groups::GroupPackage {
                name: entry.name.clone(),
                version: Version::parse(&entry.version),
                repo: Some(repo.name.clone()),
                groups: entry.groups.clone(),
            });
        }
        let package_name_set: HashSet<String> = sync_entries
            .iter()
            .map(|(_, entry)| entry.name.clone())
            .collect();
        let provider_set: HashSet<String> = sync_entries
            .iter()
            .flat_map(|(_, entry)| {
                entry
                    .provides
                    .iter()
                    .map(|provide| DepConstraint::parse(provide).name)
            })
            .collect();

        let mut expanded_names = Vec::new();
        for name in &dependency_specs {
            if name.contains('/') {
                expanded_names.push(name.clone());
                continue;
            }
            match group_index.resolve_target(name, &package_name_set, &provider_set) {
                groups::TargetResolution::Group(members) => {
                    println!(":: Group {}: {} member(s)", name, members.len());
                    for member in members {
                        if !expanded_names.contains(&member.name) {
                            expanded_names.push(member.name);
                        }
                    }
                }
                _ => expanded_names.push((*name).clone()),
            }
        }

        let requirements = expanded_names
            .iter()
            .map(|spec| Requirement::parse(spec))
            .collect::<XpmResult<Vec<_>>>()?;
        let mut installed_candidates = load_installed_candidates(&local_db_dir)?;
        installed_candidates.extend(assumed.iter().cloned());
        let plan = resolve_closure_with_installed(candidates, installed_candidates, &requirements)
            .with_context(|| {
                format!(
                    "failed to resolve dependencies for: {}",
                    expanded_names.join(", ")
                )
            })?;

        // Record explicit requests that are satisfied through `provides` so
        // the user sees the substitution in both --print and normal output.
        let substitutions: Vec<(String, String, Version)> = requirements
            .iter()
            .filter(|requirement| {
                !plan
                    .iter()
                    .any(|(candidate, _, _)| candidate.name == requirement.name)
            })
            .filter_map(|requirement| {
                plan.iter()
                    .find(|(candidate, explicit, _)| {
                        *explicit
                            && candidate
                                .provides
                                .iter()
                                .any(|provide| provide.name == requirement.name)
                    })
                    .map(|(candidate, _, _)| {
                        (
                            requirement.display(),
                            candidate.name.clone(),
                            candidate.version.clone(),
                        )
                    })
            })
            .collect();

        let mut already_satisfied = 0usize;
        // Map solved candidates back to their repository entry (exact version).
        for (candidate, explicit, is_installed) in plan {
            if is_installed {
                already_satisfied += 1;
                if explicit {
                    println!(
                        "   {} {} (already installed)",
                        candidate.name, candidate.version
                    );
                }
                continue;
            }

            let selected = sync_entries
                .iter()
                .find(|(repo, entry)| {
                    entry.name == candidate.name
                        && Version::parse(&entry.version) == candidate.version
                        && candidate
                            .repo
                            .as_deref()
                            .is_none_or(|selected_repo| repo.name == selected_repo)
                })
                .cloned();
            let Some((repo, entry)) = selected else {
                return Err(XpmError::PackageNotFound {
                    name: candidate.name.clone(),
                }
                .into());
            };
            resolved.push((repo, entry, explicit));
        }

        let explicit_count = resolved.iter().filter(|(_, _, explicit)| *explicit).count();
        println!(
            ":: Resolved {} package(s): {} explicit, {} as dependencies, {} already satisfied",
            resolved.len(),
            explicit_count,
            resolved.len() - explicit_count,
            already_satisfied
        );
        for (requested, actual, version) in &substitutions {
            println!("   requested {requested} -> {actual} {version} (provides)");
        }
    }

    if args.print {
        for (_, entry, _) in &resolved {
            println!("{} {}", entry.name, entry.version);
        }
        for spec in &local_files {
            println!("{spec}");
        }
        return Ok(None);
    }

    if resolved.is_empty() && local_files.is_empty() {
        println!(":: Nothing to do.");
        return Ok(None);
    }

    Ok(Some(InstallPlan {
        resolved,
        local_metadata,
        assumed,
        arch,
    }))
}
