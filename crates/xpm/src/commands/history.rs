//! Transaction history, diffs and rollback.

use super::*;

pub(super) fn cmd_history(config: &XpmConfig, args: &cli::HistoryArgs) -> Result<()> {
    let entries =
        Journal::list(&journal_dir(config)).context("failed to read the transaction journal")?;
    if entries.is_empty() {
        println!(":: No transactions recorded.");
        return Ok(());
    }
    if args.json {
        for journal in &entries {
            println!("{}", journal.to_json());
        }
        return Ok(());
    }
    for journal in &entries {
        println!("{}", journal.summary());
    }
    Ok(())
}

/// `xpm diff <generation>` — compare the live local database against the
/// `packages.tsv` capture of a generation.
pub(super) fn cmd_diff(config: &XpmConfig, args: &cli::DiffArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let installed = read_installed_versions(&local_db_dir)?;

    let generation = if args.generation == "current" {
        generations::read_current(&config.options.root_dir).ok_or_else(|| {
            XpmError::Other(
                "no current generation found; pass an explicit generation id (see `x gen list`)"
                    .to_string(),
            )
        })?
    } else {
        args.generation.clone()
    };

    let captured = generations::read_generation_packages(&config.options.root_dir, &generation)
        .with_context(|| format!("failed to read generation '{generation}'"))?;

    let diff = generations::diff_installed(&installed, &captured, &generation);

    if args.json {
        println!("{}", diff.to_json());
        return Ok(());
    }

    if diff.is_empty() {
        println!(":: No differences vs generation {generation}.");
        return Ok(());
    }

    if !diff.added.is_empty() {
        println!(
            ":: Added since generation {generation} ({}):",
            diff.added.len()
        );
        for pkg in &diff.added {
            println!("   + {} {}", pkg.name, pkg.version);
        }
    }
    if !diff.removed.is_empty() {
        println!(
            ":: Removed since generation {generation} ({}):",
            diff.removed.len()
        );
        for pkg in &diff.removed {
            println!("   - {} {}", pkg.name, pkg.version);
        }
    }
    if !diff.changed.is_empty() {
        println!(
            ":: Changed since generation {generation} ({}):",
            diff.changed.len()
        );
        for change in &diff.changed {
            println!("   ~ {} {} -> {}", change.name, change.from, change.to);
        }
    }
    Ok(())
}

/// Prints a rollback plan with indentation.
pub(super) fn print_rollback_plan(plan: &RollbackPlan) {
    for op in &plan.ops {
        println!("   {}", op.describe());
    }
    for (name, version) in &plan.missing {
        println!("   MISSING {name} {version} (not in cache)");
    }
    for skipped in &plan.skipped {
        println!("   skipped {skipped}");
    }
}

/// `xpm rollback [--last|--journal ID] [--dry-run]` — undo the package changes
/// of a successful transaction from the local cache.
pub(super) fn cmd_rollback(
    config: &XpmConfig,
    args: &cli::RollbackArgs,
    no_confirm: bool,
) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let entries =
        Journal::list(&journal_dir(config)).context("failed to read the transaction journal")?;

    let journal = if let Some(id) = &args.journal {
        entries
            .iter()
            .find(|entry| &entry.id == id)
            .ok_or_else(|| {
                XpmError::Other(format!("journal '{id}' not found (see `xpm history`)"))
            })?
    } else {
        last_rollback_candidate(&entries).ok_or_else(|| {
            XpmError::Other(
                "no successful install/remove/upgrade transaction to roll back".to_string(),
            )
        })?
    };

    if journal.result != "ok" {
        anyhow::bail!(
            "journal '{}' is '{}': only successful transactions can be rolled back",
            journal.id,
            journal.result
        );
    }

    let plan = xpm_core::rollback::build_plan(journal, &local_db_dir, &config.options.cache_dir)
        .context("failed to compute the rollback plan")?;

    println!(
        ":: Rollback of journal {} ({}){}",
        plan.journal_id,
        plan.action,
        plan.generation
            .as_deref()
            .map(|g| format!(", gen:{g}"))
            .unwrap_or_default()
    );

    if args.dry_run {
        print_rollback_plan(&plan);
        return Ok(());
    }

    if plan.ops.is_empty() && plan.missing.is_empty() {
        println!(":: Nothing to do.");
        return Ok(());
    }

    if !plan.is_executable() {
        print_rollback_plan(&plan);
        let missing = plan
            .missing
            .iter()
            .map(|(name, version)| format!("{name} {version}"))
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "rollback cannot proceed: package file(s) not in {}: {missing}\n\
             Re-download them with `xpm install name=version`, or use `x gen rollback` for a full system rollback.",
            config.options.cache_dir.display()
        );
    }

    confirm_action(":: Proceed with rollback? [y/N] ", no_confirm)?;

    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(
        config.options.shell_integration && config.options.root_dir != Path::new("/"),
    );
    tx.set_scriptlet_mode(config.options.scriptlet_mode);

    let mut journal_pkgs = Vec::new();
    let mut reinstall_records: Vec<InstalledRecord> = Vec::new();

    let mut transaction_paths: HashSet<String> = HashSet::new();

    for op in &plan.ops {
        match op {
            RollbackOp::Remove { name, version } => {
                tx.add_remove(name.clone())
                    .context("failed to add remove op to rollback transaction")?;
                journal_pkgs.push(JournalPackage::remove(
                    name.clone(),
                    version.clone().unwrap_or_default(),
                ));
            }
            RollbackOp::Reinstall {
                name,
                version,
                file,
            } => {
                let metadata = read_metadata(file)
                    .with_context(|| format!("failed to read metadata from {}", file.display()))?;
                transaction_paths.extend(metadata.files.iter().cloned());
                let current = local_db::read_version(&local_db_dir, name);

                match current.as_deref().filter(|current| *current != version) {
                    // Downgrade of an installed package: config-safe upgrade op.
                    Some(current_version) => {
                        tx.add_upgrade(
                            name.clone(),
                            current_version.to_string(),
                            version.clone(),
                            file.clone(),
                        )
                        .context("failed to add downgrade op to rollback transaction")?;
                        journal_pkgs.push(JournalPackage::upgrade(
                            name.clone(),
                            current_version,
                            version.clone(),
                        ));
                    }
                    // Undo of a remove: plain install from the cache.
                    None => {
                        tx.add_install(name.clone(), version.clone(), file.clone())
                            .context("failed to add install op to rollback transaction")?;
                        journal_pkgs.push(JournalPackage::install(name.clone(), version.clone()));
                    }
                }

                let reason = if current.is_some() {
                    InstallReason::read_optional(&local_db_dir, name)
                        .unwrap_or(InstallReason::Explicit)
                } else {
                    InstallReason::Explicit
                };
                reinstall_records.push(InstalledRecord {
                    name: name.clone(),
                    repo: "cache".to_string(),
                    depends: metadata.meta.depends.clone(),
                    provides: metadata.meta.provides.clone(),
                    groups: metadata.meta.groups.clone(),
                    reason,
                });
            }
        }
    }

    tx.set_transaction_paths(Arc::new(transaction_paths));
    commit_transaction(config, "rollback", journal_pkgs, Vec::new(), &mut tx)?;

    for record in &reinstall_records {
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

    println!(":: Rollback complete ({} operation(s)).", plan.ops.len());
    Ok(())
}
