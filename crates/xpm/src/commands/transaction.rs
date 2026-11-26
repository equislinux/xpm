//! Transaction orchestration shared by install, upgrade, remove and rollback.

use super::*;

pub(super) fn journal_dir(config: &XpmConfig) -> PathBuf {
    config.options.db_path.join("journal")
}

pub(super) fn hooks_dir() -> PathBuf {
    std::env::var_os("XPM_HOOKS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/xpm/hooks"))
}

pub(super) fn run_phase_hooks(
    config: &XpmConfig,
    journal: &Journal,
    phase: &str,
    action: &str,
    abort: bool,
) -> Result<()> {
    let dir = hooks_dir().join(format!("{phase}-transaction.d"));
    let envs = vec![
        (
            "XPM_ROOT_DIR".to_string(),
            config.options.root_dir.display().to_string(),
        ),
        ("XPM_ACTION".to_string(), action.to_string()),
        (
            "XPM_JOURNAL".to_string(),
            journal.path.display().to_string(),
        ),
        (
            "XPM_PKG_NAMES".to_string(),
            journal
                .packages
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        (
            "XPM_PKG_VERSIONS".to_string(),
            journal
                .packages
                .iter()
                .map(|p| p.to.clone().unwrap_or_default())
                .collect::<Vec<_>>()
                .join(" "),
        ),
    ];
    let outcome = run_transaction_hooks(&dir, &envs)
        .with_context(|| format!("failed to run {phase}-transaction hooks"))?;
    for hook in &outcome.ran {
        tracing::debug!(hook = %hook, "transaction hook ran");
    }
    if !outcome.failed.is_empty() {
        let list = outcome.failed.join(", ");
        if abort {
            anyhow::bail!("{phase}-transaction hook(s) failed: {list}");
        }
        eprintln!("xpm: warning: {phase}-transaction hook(s) failed: {list}");
    }
    Ok(())
}

/// Runs the pacman-style ALPM `.hook` files for one phase of a transaction.
///
/// This is what makes xpm transactions generation-aware: the distribution
/// ships `10-x-gen-pre.hook`/`20-x-gen-post.hook` in `/etc/pacman.d/hooks`.
pub(super) fn hook_paths_from_targets(
    targets: &[conflict::TargetManifest],
) -> Vec<(String, HookOperation)> {
    let mut paths = Vec::new();
    for target in targets {
        if target.old_files.is_empty() {
            for file in &target.files {
                paths.push((file.clone(), HookOperation::Install));
            }
            continue;
        }

        let old: std::collections::HashSet<&str> =
            target.old_files.iter().map(String::as_str).collect();
        let new: std::collections::HashSet<&str> =
            target.files.iter().map(String::as_str).collect();

        for file in &target.files {
            if !old.contains(file.as_str()) {
                paths.push((file.clone(), HookOperation::Upgrade));
            }
        }
        for file in &target.old_files {
            if !new.contains(file.as_str()) {
                paths.push((file.clone(), HookOperation::Upgrade));
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn hook_paths_for_removals(
    local_db_dir: &Path,
    names: &[String],
) -> Result<Vec<(String, HookOperation)>> {
    let mut paths = Vec::new();
    for name in names {
        for file in local_db::read_files(local_db_dir, name)? {
            paths.push((file, HookOperation::Remove));
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(super) fn run_alpm_hooks(
    config: &XpmConfig,
    packages: &[JournalPackage],
    paths: &[(String, HookOperation)],
    when: HookWhen,
) -> Result<()> {
    if !config.options.alpm_hooks {
        tracing::debug!("ALPM hooks disabled for this transaction");
        return Ok(());
    }

    let transaction = HookTransaction {
        packages: packages
            .iter()
            .map(|pkg| {
                let operation = match (&pkg.from, &pkg.to) {
                    (None, Some(_)) => HookOperation::Install,
                    (Some(_), None) => HookOperation::Remove,
                    _ => HookOperation::Upgrade,
                };
                (pkg.name.as_str(), operation)
            })
            .collect(),
        paths: paths.to_vec(),
    };

    let hooks = alpm_hooks::load_hooks(&alpm_hooks::hook_dirs_for(&config.options.root_dir))
        .context("failed to load ALPM hooks")?;
    if hooks.is_empty() {
        return Ok(());
    }

    let installed = alpm_hooks::installed_names(&config.options.db_path.join("local"));
    let failures = alpm_hooks::run_hooks(
        &hooks,
        when,
        &transaction,
        &config.options.root_dir,
        &config.options.db_path,
        |name| installed.contains(name),
    )?;
    let warnings = alpm_hooks::enforce_failures(when, &failures)?;
    for warning in warnings {
        eprintln!("xpm: warning: ALPM hook failed: {warning}");
    }
    Ok(())
}

/// Prepares, commits and journals a transaction, running the hook
/// directories around it.
pub(super) fn commit_transaction(
    config: &XpmConfig,
    action: &str,
    packages: Vec<JournalPackage>,
    hook_paths: Vec<(String, HookOperation)>,
    tx: &mut Transaction,
) -> Result<()> {
    let mut journal = Journal::start(
        &journal_dir(config),
        action,
        &config.options.root_dir,
        packages,
    )
    .context("failed to start the transaction journal")?;

    let result = (|| -> Result<()> {
        run_phase_hooks(config, &journal, "pre", action, true)?;
        run_alpm_hooks(
            config,
            &journal.packages,
            &hook_paths,
            HookWhen::PreTransaction,
        )?;
        println!(
            ":: Preparing transaction ({} operation(s))...",
            tx.operation_count()
        );
        tx.prepare().context("transaction preparation failed")?;
        println!(":: Committing transaction...");
        tx.commit().context("transaction commit failed")?;
        run_phase_hooks(config, &journal, "post", action, false)?;
        run_alpm_hooks(
            config,
            &journal.packages,
            &hook_paths,
            HookWhen::PostTransaction,
        )?;
        Ok(())
    })();

    match &result {
        Ok(()) => {
            // Link the finished transaction to the generation that is current
            // after the post hooks (the generation engine ships one that
            // snapshots here). Absent/unreadable state leaves `None`.
            journal.generation = generations::read_current(&config.options.root_dir);
            journal
                .finish("ok", None)
                .context("failed to finalize the transaction journal")?;
        }
        Err(e) => {
            let _ = journal.finish("failed", Some(format!("{e:#}")));
        }
    }
    result
}
