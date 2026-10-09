//! xpm — Modern package manager for X Distribution
//!
//! Entry point for the xpm binary. Handles CLI parsing, configuration loading,
//! logging initialization, and dispatching to the appropriate subcommand handler.

mod cli;

use anyhow::{Context, Result};
use clap::Parser;
use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use tracing::Level;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use xpm_core::alpm_hooks::{self, HookOperation, HookTransaction, HookWhen};
use xpm_core::config::Repository;
use xpm_core::generations;
use xpm_core::install_reason::{retain_by_reason, InstallReason};
use xpm_core::journal::{Journal, JournalPackage};
use xpm_core::local_db;
use xpm_core::orphans::{find_orphans, InstalledPackage};
use xpm_core::package::read_metadata;
use xpm_core::repo::RepoManager;
use xpm_core::repo_db::{merge_files_db, parse_sync_db, RepoEntry};
use xpm_core::repo_sync::{
    download_first_available, package_download_candidates, sync_repo_databases,
    verify_remote_signature, verify_sha256,
};
use xpm_core::resolver::{resolve_closure, DepConstraint, PackageCandidate, Requirement, Version};
use xpm_core::rollback::{last_rollback_candidate, RollbackOp, RollbackPlan};
use xpm_core::txhooks::run_transaction_hooks;
use xpm_core::{HookChain, Transaction};
use xpm_core::{XpmConfig, XpmError, XpmResult};

fn main() -> Result<()> {
    // Rust ignores SIGPIPE by default, so `xpm ... | head` panics with a
    // broken-pipe error; restore the default handler like other CLIs.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();

    // ── Initialize logging ──────────────────────────────────────────────
    let log_level = match cli.verbose {
        0 => Level::WARN,
        1 => Level::INFO,
        2 => Level::DEBUG,
        _ => Level::TRACE,
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(log_level.into())
                .from_env_lossy(),
        )
        .with_target(false)
        .init();

    tracing::debug!("xpm v{}", env!("CARGO_PKG_VERSION"));

    // ── Load configuration ──────────────────────────────────────────────
    let config_path = cli.config.clone().unwrap_or_else(XpmConfig::default_path);

    let mut config = XpmConfig::load_or_default(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    // Apply CLI overrides
    config.apply_overrides(
        cli.root.as_deref(),
        cli.dbpath.as_deref(),
        cli.cachedir.as_deref(),
    );

    if cli.no_color {
        config.options.color = false;
    }

    tracing::info!(
        root = %config.options.root_dir.display(),
        db = %config.options.db_path.display(),
        repos = config.repositories.len(),
        "configuration loaded"
    );

    // ── Dispatch subcommands ────────────────────────────────────────────
    match &cli.command {
        Command::Sync(args) => cmd_sync(&config, args),
        Command::Install(args) => cmd_install(&config, args, cli.no_confirm),
        Command::Remove(args) => cmd_remove(&config, args, cli.no_confirm),
        Command::Upgrade(args) => cmd_upgrade(&config, args, cli.no_confirm),
        Command::Query(args) => cmd_query(&config, args),
        Command::Search(args) => cmd_search(&config, args),
        Command::Info(args) => cmd_info(&config, args),
        Command::Files(args) => cmd_files(&config, args),
        Command::Repo(args) => cmd_repo(&config, args),
        Command::History(args) => cmd_history(&config, args),
        Command::Rollback(args) => cmd_rollback(&config, args, cli.no_confirm),
        Command::Diff(args) => cmd_diff(&config, args),
        Command::Usage(args) => cmd_help(args),
    }
}

// ── Subcommand stubs ────────────────────────────────────────────────────────
//
// Each function below is a placeholder that will be filled with real logic
// in subsequent phases. For now they confirm the CLI pipeline works end-to-end.

fn cmd_sync(config: &XpmConfig, args: &cli::SyncArgs) -> Result<()> {
    let force = if args.force { " (forced)" } else { "" };
    println!(":: Synchronizing package databases{force}...");

    let arch = config
        .options
        .architecture
        .clone()
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let sync_dir = config.options.db_path.join("sync");

    let remote_results = sync_repositories_in_parallel(
        &config.repositories,
        &arch,
        &sync_dir,
        3,
        config.options.parallel_downloads.max(1) as usize,
        config.options.sig_level,
        config.options.keyring_path(),
    );
    let mut remote_by_repo: HashMap<String, Result<xpm_core::repo_sync::RepoSyncResult, XpmError>> =
        remote_results.into_iter().collect();

    for repo in &config.repositories {
        println!("   {} — {} server(s)", repo.name, repo.server.len());

        match remote_by_repo
            .remove(&repo.name)
            .unwrap_or_else(|| Err(XpmError::Other("missing remote sync result".to_string())))
        {
            Ok(result) => {
                println!("     mirror: {}", result.mirror);
                if result.db_downloaded {
                    println!("     remote: {}.db updated", repo.name);
                }
                if result.files_downloaded {
                    println!("     remote: {}.files updated", repo.name);
                }
            }
            Err(err) => {
                tracing::warn!(repo = %repo.name, error = %err, "remote sync failed");
                println!("     remote: unavailable ({err})");
            }
        }

        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if db_path.exists() {
            match parse_sync_db(&db_path, &repo.name) {
                Ok(mut db) => {
                    let files_path = sync_dir.join(format!("{}.files", repo.name));
                    if files_path.exists() {
                        if let Err(err) = merge_files_db(&files_path, &mut db) {
                            tracing::warn!(
                                repo = %repo.name,
                                path = %files_path.display(),
                                error = %err,
                                "failed to parse .files database"
                            );
                        }
                    }

                    let with_files = db.entries.iter().filter(|e| !e.files.is_empty()).count();
                    println!(
                        "     local db: {} package(s) loaded ({} with file lists)",
                        db.entries.len(),
                        with_files
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        repo = %repo.name,
                        path = %db_path.display(),
                        error = %err,
                        "failed to parse local sync database"
                    );
                    println!("     local db: parse error ({err})");
                }
            }
        } else {
            println!(
                "     local db: not found at {}",
                display_rel_or_abs(&db_path)
            );
        }
    }
    println!(":: Sync complete.");
    Ok(())
}

fn display_rel_or_abs(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(|p| p.display().to_string()))
        .unwrap_or_else(|| path.display().to_string())
}

fn sync_repositories_in_parallel(
    repositories: &[Repository],
    arch: &str,
    sync_dir: &Path,
    retries: u32,
    max_parallel: usize,
    default_sig_level: xpm_core::config::SigLevel,
    keyring_path: PathBuf,
) -> Vec<(
    String,
    Result<xpm_core::repo_sync::RepoSyncResult, XpmError>,
)> {
    let mut results = Vec::with_capacity(repositories.len());

    for chunk in repositories.chunks(max_parallel.max(1)) {
        let mut handles = Vec::with_capacity(chunk.len());

        for repo in chunk {
            let repo_clone = repo.clone();
            let arch_owned = arch.to_string();
            let sync_dir_owned = sync_dir.to_path_buf();
            let keyring_path_owned = keyring_path.clone();

            handles.push(thread::spawn(move || {
                let name = repo_clone.name.clone();
                let result = sync_repo_databases(
                    &repo_clone,
                    &arch_owned,
                    &sync_dir_owned,
                    retries,
                    default_sig_level,
                    &keyring_path_owned,
                );
                (name, result)
            }));
        }

        for handle in handles {
            match handle.join() {
                Ok(result) => results.push(result),
                Err(_) => results.push((
                    "unknown".to_string(),
                    Err(XpmError::Other("sync worker thread panicked".to_string())),
                )),
            }
        }
    }

    results
}

fn confirm_action(prompt: &str, no_confirm: bool) -> Result<()> {
    if no_confirm {
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        return Err(XpmError::Other(
            "confirmation required but stdin is not interactive; use --no-confirm".to_string(),
        )
        .into());
    }

    print!("{}", prompt);
    io::stdout().flush().context("failed to flush prompt")?;

    let mut input = String::new();
    let bytes = io::stdin()
        .read_line(&mut input)
        .context("failed to read confirmation")?;

    if bytes == 0 {
        return Err(XpmError::Other(
            "confirmation prompt received EOF; use --no-confirm for non-interactive mode"
                .to_string(),
        )
        .into());
    }

    let answer = input.trim().to_ascii_lowercase();
    if answer == "y" || answer == "yes" {
        Ok(())
    } else {
        Err(XpmError::Other("operation cancelled by user".to_string()).into())
    }
}

/// Build a resolver candidate from a sync database entry.
fn resolver_candidate(entry: &RepoEntry) -> PackageCandidate {
    PackageCandidate {
        name: entry.name.clone(),
        version: Version::parse(&entry.version),
        depends: entry
            .depends
            .iter()
            .map(|d| DepConstraint::parse(d))
            .collect(),
        conflicts: entry
            .conflicts
            .iter()
            .map(|c| DepConstraint::parse(c))
            .collect(),
        provides: entry
            .provides
            .iter()
            .map(|p| DepConstraint::parse(p))
            .collect(),
        optdepends: entry.opt_depends.clone(),
    }
}

/// Local-database metadata recorded for every package installed in a run.
struct InstalledRecord {
    name: String,
    repo: String,
    depends: Vec<String>,
    provides: Vec<String>,
    reason: InstallReason,
}

fn cmd_install(config: &XpmConfig, args: &cli::InstallArgs, no_confirm: bool) -> Result<()> {
    if args.as_deps && args.as_explicit {
        anyhow::bail!("xpm install: --as-deps and --as-explicit are mutually exclusive");
    }

    let (local_files, package_names): (Vec<&String>, Vec<&String>) = args
        .packages
        .iter()
        .partition(|spec| Path::new(spec.as_str()).is_file());

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
    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create cache dir {}", cache_dir.display()))?;

    // ── Resolve repository packages (if any) ────────────────────────
    let mut resolved: Vec<(Repository, RepoEntry, bool)> = Vec::new();
    if !package_names.is_empty() {
        let mut candidates = Vec::new();
        let mut sync_entries: Vec<(Repository, RepoEntry)> = Vec::new();
        for repo in &config.repositories {
            let db_path = sync_dir.join(format!("{}.db", repo.name));
            if !db_path.exists() {
                continue;
            }
            let db = parse_sync_db(&db_path, &repo.name).with_context(|| {
                format!("failed to parse sync db {}", display_rel_or_abs(&db_path))
            })?;
            for entry in db.entries {
                candidates.push(resolver_candidate(&entry));
                sync_entries.push((repo.clone(), entry));
            }
        }

        let requirements = package_names
            .iter()
            .map(|spec| Requirement::parse(spec))
            .collect::<XpmResult<Vec<_>>>()?;
        let plan = resolve_closure(candidates, &requirements).with_context(|| {
            format!(
                "failed to resolve dependencies for: {}",
                package_names
                    .iter()
                    .map(|spec| spec.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        // Map solved candidates back to their repository entry (exact version).
        for (candidate, explicit) in plan {
            let selected = sync_entries
                .iter()
                .find(|(_, entry)| {
                    entry.name == candidate.name
                        && Version::parse(&entry.version) == candidate.version
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
            ":: Resolved {} package(s): {} explicit, {} as dependencies",
            resolved.len(),
            explicit_count,
            resolved.len() - explicit_count
        );
    }

    // Create transaction
    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;

    // Setup hooks chain
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(config.options.root_dir != Path::new("/"));

    let mut journal_pkgs = Vec::new();
    let mut installed: Vec<InstalledRecord> = Vec::new();

    // Phase 1a: local package files (already "downloaded")
    for spec in &local_files {
        let path = Path::new(spec.as_str());
        let metadata = read_metadata(path)
            .with_context(|| format!("failed to read metadata from {}", path.display()))?;
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

        let pkg_reason = if args.as_deps {
            InstallReason::Dep
        } else {
            InstallReason::Explicit
        };

        println!("   local: {} ({} {})", path.display(), pkg_name, version);

        journal_pkgs.push(JournalPackage::install(&pkg_name, &version));
        installed.push(InstalledRecord {
            name: pkg_name.clone(),
            repo: "local".to_string(),
            depends: metadata.meta.depends.clone(),
            provides: metadata.meta.provides.clone(),
            reason: pkg_reason,
        });

        tx.add_install(pkg_name, version, path.to_path_buf())
            .context("failed to add local install to transaction")?;
    }

    // Phase 1b: Download and validate the resolved plan (dependencies first)
    for (repo, entry, explicit) in &resolved {
        let pkg_name = &entry.name;

        let filename = entry.resolved_filename(&arch);
        let dest = cache_dir.join(&filename);
        let urls = package_download_candidates(repo, &arch, entry);
        let mirror = download_first_available(&urls, &dest, 3).with_context(|| {
            format!(
                "failed to download '{}' from repo '{}'",
                pkg_name, repo.name
            )
        })?;

        let sig_level = repo.sig_level.unwrap_or(config.options.sig_level);
        let keyring_path = config.options.keyring_path();
        let sig_url = format!("{mirror}.sig");
        verify_remote_signature(&dest, &sig_url, sig_level, &keyring_path, 3).with_context(
            || {
                format!(
                    "signature verification failed for '{}' from repo '{}'",
                    pkg_name, repo.name
                )
            },
        )?;

        if let Some(sum) = entry.sha256sum.as_deref() {
            verify_sha256(&dest, sum)?;
        }

        let metadata = read_metadata(&dest)
            .with_context(|| format!("failed to read metadata from {}", dest.display()))?;

        let pkg_reason = if args.as_deps {
            InstallReason::Dep
        } else if args.as_explicit || *explicit {
            InstallReason::Explicit
        } else {
            InstallReason::Dep
        };

        println!("   downloaded: {}", dest.display());
        println!("   source: {}", mirror);

        let mut journal_pkg = JournalPackage::install(&entry.name, &entry.version)
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
            reason: pkg_reason,
        });

        // Add to transaction
        tx.add_install(entry.name.clone(), entry.version.clone(), dest)
            .context("failed to add install to transaction")?;
    }

    if args.download_only {
        println!(":: Download complete.");
        return Ok(());
    }

    confirm_action(
        ":: Proceed with installation? [y/N] (download already completed) ",
        no_confirm,
    )?;

    // Phase 2/3: prepare, commit, hooks and journal.
    commit_transaction(config, "install", journal_pkgs, &mut tx)?;

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
    }

    println!(":: {} package(s) installed successfully.", installed.len());
    if config.options.root_dir != Path::new("/") {
        println!(":: Shell integration enabled via ~/.local/bin shims.");
        println!(":: If this shell does not find new commands yet, run: hash -r");
        println!(":: For immediate PATH refresh, run: source ~/.zshrc or source ~/.bashrc");
    }
    Ok(())
}

fn cmd_remove(config: &XpmConfig, args: &cli::RemoveArgs, no_confirm: bool) -> Result<()> {
    println!(":: Removing packages: {}", args.packages.join(", "));
    if args.recursive {
        println!("   (including unneeded dependencies)");
    }

    // Create transaction
    let local_db_dir = config.options.db_path.join("local");
    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;

    // Setup hooks chain
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(config.options.root_dir != Path::new("/"));
    tx.set_save_configs(!args.nosave);

    // Add remove operations for each package
    let mut journal_pkgs = Vec::new();
    for pkg_name in &args.packages {
        // Verify package is installed
        let pkg_dir = local_db_dir.join(pkg_name);
        if !pkg_dir.exists() {
            return Err(
                XpmError::Package(format!("package '{}' is not installed", pkg_name)).into(),
            );
        }

        let version = std::fs::read_to_string(pkg_dir.join("version"))
            .unwrap_or_default()
            .trim()
            .to_string();
        journal_pkgs.push(JournalPackage::remove(pkg_name, version));

        tx.add_remove(pkg_name.clone())
            .context("failed to add remove to transaction")?;
    }

    confirm_action(":: Proceed with removal? [y/N] ", no_confirm)?;

    // Phase 2/3: prepare, commit, hooks and journal.
    commit_transaction(config, "remove", journal_pkgs, &mut tx)?;

    println!(
        ":: {} package(s) removed successfully.",
        args.packages.len()
    );
    if config.options.root_dir != Path::new("/") {
        println!(":: If command lookup is stale in current shell, run: hash -r");
    }
    Ok(())
}

fn cmd_upgrade(config: &XpmConfig, args: &cli::UpgradeArgs, no_confirm: bool) -> Result<()> {
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

    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create cache dir {}", cache_dir.display()))?;

    let installed = read_installed_versions(&local_db_dir)?;
    if installed.is_empty() {
        println!(":: No packages are currently installed in xpm local database.");
        return Ok(());
    }

    let remote_latest = read_latest_remote_entries(config, &sync_dir)?;

    let mut planned = Vec::new();
    for (pkg_name, local_version) in &installed {
        if args.ignore.iter().any(|i| i == pkg_name) {
            continue;
        }

        let Some((repo, entry)) = remote_latest.get(pkg_name) else {
            continue;
        };

        let ordering = Version::cmp_versions(&entry.version, local_version);
        if ordering.is_gt() || (args.force && ordering.is_eq()) {
            planned.push((repo.clone(), entry.clone(), local_version.clone()));
        }
    }

    if planned.is_empty() {
        println!(":: Nothing to do. System is up to date.");
        return Ok(());
    }

    // Include the transitive closure of the upgraded packages so new or
    // newly-required dependencies are installed in the same run.
    let planned_names: HashSet<String> = planned
        .iter()
        .map(|(_, entry, _)| entry.name.clone())
        .collect();
    let mut candidates = Vec::new();
    for (_, entry) in remote_latest.values() {
        candidates.push(resolver_candidate(entry));
    }
    let requirements: Vec<Requirement> = planned_names
        .iter()
        .map(|name| Requirement {
            name: name.clone(),
            version: None,
        })
        .collect();
    let closure = resolve_closure(candidates, &requirements).with_context(|| {
        format!(
            "failed to resolve the upgrade closure for: {}",
            planned_names.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    })?;

    let mut plan: Vec<(Repository, RepoEntry, String, bool)> = Vec::new();
    for (candidate, _explicit) in closure {
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
                    plan.push((repo.clone(), entry.clone(), local.clone(), false));
                }
            }
            None => plan.push((repo.clone(), entry.clone(), String::new(), true)),
        }
    }

    let upgrade_count = plan.iter().filter(|(_, _, _, is_new)| !*is_new).count();
    let new_dep_count = plan.iter().filter(|(_, _, _, is_new)| *is_new).count();
    if new_dep_count > 0 {
        println!(":: Packages to upgrade: {upgrade_count} (+{new_dep_count} new dependency/ies)");
    } else {
        println!(":: Packages to upgrade: {upgrade_count}");
    }
    for (_, entry, local_version, is_new) in &plan {
        if *is_new {
            println!("   {} {} (new dependency)", entry.name, entry.version);
        } else {
            println!("   {} {} -> {}", entry.name, local_version, entry.version);
        }
    }

    confirm_action(":: Proceed with upgrade? [y/N] ", no_confirm)?;

    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(config.options.root_dir != Path::new("/"));

    let mut journal_pkgs = Vec::new();
    let mut installed_records: Vec<InstalledRecord> = Vec::new();

    for (repo, entry, local_version, is_new) in plan {
        let filename = entry.resolved_filename(&arch);

        let dest = cache_dir.join(&filename);
        let urls = package_download_candidates(&repo, &arch, &entry);
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

        if let Some(sum) = entry.sha256sum.as_deref() {
            verify_sha256(&dest, sum)?;
        }

        let metadata = read_metadata(&dest)
            .with_context(|| format!("failed to read metadata from {}", dest.display()))?;

        // New dependency: plain install. Upgrade of an existing package: a
        // single `Upgrade` operation, so hooks can compare the old and new
        // manifests (config files, stale files) before the local-db entry is
        // rewritten.
        let mut journal_pkg = if is_new {
            tx.add_install(entry.name.clone(), entry.version.clone(), dest)
                .context("failed to add install op to transaction")?;
            JournalPackage::install(&entry.name, &entry.version)
        } else {
            tx.add_upgrade(
                entry.name.clone(),
                local_version.clone(),
                entry.version.clone(),
                dest,
            )
            .context("failed to add upgrade op to transaction")?;
            JournalPackage::upgrade(&entry.name, &local_version, &entry.version)
        };
        journal_pkg = journal_pkg.with_repo(&repo.name).with_source(&mirror);
        if let Some(sum) = entry.sha256sum.as_deref() {
            journal_pkg = journal_pkg.with_sha256(sum);
        }
        journal_pkgs.push(journal_pkg);

        let record_reason = if is_new {
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
            reason: record_reason,
        });
    }

    commit_transaction(config, "upgrade", journal_pkgs, &mut tx)?;

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
    }

    println!(":: Upgrade complete.");
    Ok(())
}

fn read_installed_versions(local_db_dir: &Path) -> Result<HashMap<String, String>> {
    let mut installed = HashMap::new();

    if !local_db_dir.exists() {
        return Ok(installed);
    }

    for entry in std::fs::read_dir(local_db_dir)
        .with_context(|| format!("failed to read {}", local_db_dir.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() {
            continue;
        }

        let pkg_name = entry.file_name().to_string_lossy().into_owned();
        let version_path = entry.path().join("version");
        if !version_path.exists() {
            continue;
        }

        let version = std::fs::read_to_string(&version_path)
            .with_context(|| format!("failed to read {}", version_path.display()))?
            .trim()
            .to_string();

        if !version.is_empty() {
            installed.insert(pkg_name, version);
        }
    }

    Ok(installed)
}

fn read_latest_remote_entries(
    config: &XpmConfig,
    sync_dir: &Path,
) -> Result<HashMap<String, (Repository, xpm_core::repo_db::RepoEntry)>> {
    let mut latest = HashMap::new();

    for repo in &config.repositories {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            continue;
        }

        let db = parse_sync_db(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", db_path.display()))?;

        // Preserve repository priority order: first repo that contains a package wins.
        let mut best_by_name: HashMap<String, xpm_core::repo_db::RepoEntry> = HashMap::new();
        for entry in db.entries {
            match best_by_name.get(&entry.name) {
                Some(existing) => {
                    if Version::cmp_versions(&entry.version, &existing.version).is_gt() {
                        best_by_name.insert(entry.name.clone(), entry);
                    }
                }
                None => {
                    best_by_name.insert(entry.name.clone(), entry);
                }
            }
        }

        for (name, entry) in best_by_name {
            latest.entry(name).or_insert_with(|| (repo.clone(), entry));
        }
    }

    Ok(latest)
}

fn journal_dir(config: &XpmConfig) -> PathBuf {
    config.options.db_path.join("journal")
}

fn hooks_dir() -> PathBuf {
    std::env::var_os("XPM_HOOKS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/usr/lib/xpm/hooks"))
}

fn run_phase_hooks(
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
fn run_alpm_hooks(config: &XpmConfig, packages: &[JournalPackage], when: HookWhen) -> Result<()> {
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
        paths: Vec::new(),
    };

    let hooks =
        alpm_hooks::load_hooks(&alpm_hooks::hook_dirs()).context("failed to load ALPM hooks")?;
    if hooks.is_empty() {
        return Ok(());
    }

    let installed = alpm_hooks::installed_names(&config.options.db_path.join("local"));
    let failures =
        alpm_hooks::run_hooks(&hooks, when, &transaction, |name| installed.contains(name))?;
    let warnings = alpm_hooks::enforce_failures(when, &failures)?;
    for warning in warnings {
        eprintln!("xpm: warning: ALPM hook failed: {warning}");
    }
    Ok(())
}

/// Prepares, commits and journals a transaction, running the hook
/// directories around it.
fn commit_transaction(
    config: &XpmConfig,
    action: &str,
    packages: Vec<JournalPackage>,
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
        run_alpm_hooks(config, &journal.packages, HookWhen::PreTransaction)?;
        println!(
            ":: Preparing transaction ({} operation(s))...",
            tx.operation_count()
        );
        tx.prepare().context("transaction preparation failed")?;
        println!(":: Committing transaction...");
        tx.commit().context("transaction commit failed")?;
        run_phase_hooks(config, &journal, "post", action, false)?;
        run_alpm_hooks(config, &journal.packages, HookWhen::PostTransaction)?;
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

fn cmd_history(config: &XpmConfig, args: &cli::HistoryArgs) -> Result<()> {
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
fn cmd_diff(config: &XpmConfig, args: &cli::DiffArgs) -> Result<()> {
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
fn print_rollback_plan(plan: &RollbackPlan) {
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
fn cmd_rollback(config: &XpmConfig, args: &cli::RollbackArgs, no_confirm: bool) -> Result<()> {
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
    tx.set_shell_integration(config.options.root_dir != Path::new("/"));

    let mut journal_pkgs = Vec::new();
    let mut reinstall_records: Vec<InstalledRecord> = Vec::new();

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
                    reason,
                });
            }
        }
    }

    commit_transaction(config, "rollback", journal_pkgs, &mut tx)?;

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
    }

    println!(":: Rollback complete ({} operation(s)).", plan.ops.len());
    Ok(())
}

fn cmd_query(config: &XpmConfig, args: &cli::QueryArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let installed = read_installed_versions(&local_db_dir)?;
    let mut packages: Vec<(String, String)> = installed.into_iter().collect();

    if let Some(filter) = args.filter.as_deref() {
        packages.retain(|(name, _)| name.contains(filter));
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
        let orphans: std::collections::HashSet<String> =
            find_orphans(&installed_packages).into_iter().collect();
        packages.retain(|(name, _)| orphans.contains(name));
    }

    if args.explicit && args.deps {
        anyhow::bail!("xpm query: --explicit and --deps are mutually exclusive");
    }

    if args.explicit || args.deps {
        let wanted = if args.explicit {
            InstallReason::Explicit
        } else {
            InstallReason::Dep
        };
        retain_by_reason(&local_db_dir, &mut packages, wanted);
    }

    packages.sort();
    for (name, version) in &packages {
        match args.format.as_str() {
            "tsv" => println!("{name}\t{version}"),
            _ => println!("{name} {version}"),
        }
    }
    Ok(())
}

fn cmd_search(config: &XpmConfig, args: &cli::SearchArgs) -> Result<()> {
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
            let db = parse_sync_db(&db_path, &repo.name)
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

fn cmd_info(config: &XpmConfig, args: &cli::InfoArgs) -> Result<()> {
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

fn cmd_files(config: &XpmConfig, args: &cli::FilesArgs) -> Result<()> {
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

fn cmd_repo(config: &XpmConfig, args: &cli::RepoArgs) -> Result<()> {
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

fn cmd_help(args: &cli::HelpArgs) -> Result<()> {
    match args.topic.as_deref() {
        None | Some("") => print_help_overview(),
        Some("commands") => print_help_commands(),
        Some("config") => print_help_config(),
        Some("repos") | Some("repositories") => print_help_repos(),
        Some(cmd) => print_help_command(cmd),
    }
    Ok(())
}

fn print_help_overview() {
    println!(
        r#"xpm — Modern package manager for X Distribution

USAGE:
    xpm <COMMAND> [OPTIONS]
    xpm <ALIAS> [OPTIONS]

QUICK START:
    xpm sync                Synchronize package databases
    xpm install <pkg>       Install a package
    xpm remove <pkg>        Remove a package
    xpm upgrade             Upgrade all packages
    xpm search <query>      Search for packages

TOPICS:
    xpm usage commands      List all available commands
    xpm usage config        Configuration file format
    xpm usage repos         Repository management
    xpm usage <command>     Help for a specific command

GLOBAL FLAGS:
    -c, --config <PATH>     Custom configuration file
    -v, --verbose           Increase verbosity (-v, -vv, -vvv)
    --no-confirm            Skip confirmation prompts
    --root <PATH>           Alternative installation root
    --dbpath <PATH>         Alternative database directory
    --cachedir <PATH>       Alternative cache directory
    --no-color              Disable colored output

PACMAN ALIASES:
    Sy → sync     S → install    R → remove     Su → upgrade
    Q  → query    Ss → search    Si → info      Ql → files

DOCUMENTATION:
    Full CLI reference: docs/CLI.md
    Configuration:      /etc/xpm.conf
    User repos:         /etc/xpm.d/
"#
    );
}

fn print_help_commands() {
    println!(
        r#"xpm — Available Commands

PACKAGE OPERATIONS:
    sync        Synchronize package databases from mirrors
    install     Install one or more packages
    remove      Remove installed packages
    upgrade     Upgrade all installed packages

QUERIES:
    query       Query the local package database
    search      Search for packages in sync databases
    info        Display detailed package information
    files       List files owned by a package
    history     Show the transaction journal
    rollback    Undo the last transaction (package level, from the cache)
    diff        Compare installed packages against a generation capture

REPOSITORY MANAGEMENT:
    repo add    Add a temporary repository
    repo remove Remove a user-added repository
    repo list   List all active repositories

HELP:
    usage       Display detailed usage information

For detailed help on any command:
    xpm usage <command>
    xpm <command> --help
"#
    );
}

fn print_help_config() {
    println!(
        r#"xpm — Configuration

CONFIGURATION FILE:
    /etc/xpm.conf (TOML format)

GENERAL OPTIONS:
    [options]
    root_dir = "/"                    # Installation root
    db_path = "/var/lib/xpm/"         # Database directory
    cache_dir = "/var/cache/xpm/pkg/" # Package cache
    log_file = "/var/log/xpm.log"     # Log file location
    gpg_dir = "/etc/pacman.d/gnupg/"  # GPG keyring (shared with pacman;
                                      # falls back to /etc/xpm/gnupg)
    sig_level = "optional"            # required | optional | never
    parallel_downloads = 5            # Concurrent downloads
    check_space = true                # Check disk space
    color = true                      # Colored output
    architecture = "x86_64"           # System architecture

PACKAGE LISTS:
    hold_pkg = ["linux"]              # Never upgrade these
    ignore_pkg = ["pkg1", "pkg2"]     # Skip during upgrades
    ignore_group = ["group1"]         # Skip entire groups

REPOSITORY DEFINITION:
    [[repo]]
    name = "core"
    server = [
        "https://mirror.example.com/$repo/os/$arch",
        "https://mirror2.example.com/$repo/os/$arch"
    ]
    sig_level = "required"            # Override global setting

URL VARIABLES:
    $repo   Repository name (e.g., "core", "extra")
    $arch   System architecture (e.g., "x86_64")

FILES:
    /etc/xpm.conf           Main configuration
    /etc/xpm.d/*.toml       User-added repositories
"#
    );
}

fn print_help_repos() {
    println!(
        r#"xpm — Repository Management

PREDEFINED REPOSITORIES:
    Configured in /etc/xpm.conf as [[repo]] sections.
    These are managed by the distribution maintainers.

USER-ADDED REPOSITORIES:
    Stored as individual files in /etc/xpm.d/
    Managed via `xpm repo` commands.

COMMANDS:
    xpm repo list                   List all repositories
    xpm repo add <name> <url>       Add a repository
    xpm repo remove <name>          Remove a repository

EXAMPLES:
    # Add Chaotic-AUR repository
    xpm repo add chaotic-aur https://cdn-mirror.chaotic.cx/$repo/$arch

    # Add a GitHub Pages hosted repo
    xpm repo add my-repo https://user.github.io/my-repo/$arch

    # Add a local file repository
    xpm repo add local file:///srv/packages/$arch

URL VARIABLES:
    $repo   Replaced with the repository name
    $arch   Replaced with system architecture (x86_64, aarch64)

SIGNATURE LEVELS:
    required    Signatures must be present and valid
    optional    Verify if present, allow unsigned (default)
    never       Skip verification completely

After adding a repository, run `xpm sync` to fetch its database.
"#
    );
}

fn print_help_command(cmd: &str) {
    match cmd {
        "sync" | "Sy" => println!(
            r#"xpm sync — Synchronize Package Databases

USAGE:
    xpm sync [OPTIONS]
    xpm Sy [OPTIONS]

DESCRIPTION:
    Downloads the latest package database files from all configured
    repositories. This should be run before installing or upgrading
    packages to ensure you have the latest version information.

OPTIONS:
    -f, --force     Force a full database refresh even if local
                    databases appear to be up to date

EXAMPLES:
    xpm sync            # Normal sync
    xpm sync --force    # Force full refresh
    xpm Sy -f           # Same as above
"#
        ),
        "install" | "S" => println!(
            r#"xpm install — Install Packages

USAGE:
    xpm install <PACKAGES>... [OPTIONS]
    xpm S <PACKAGES>... [OPTIONS]

DESCRIPTION:
    Install one or more packages from the synchronized databases.
    Dependencies are not resolved automatically yet; install them explicitly.

ARGUMENTS:
    <PACKAGES>      One or more package names to install

OPTIONS:
    -w, --download-only     Download packages without installing
    --as-deps               Mark as installed as a dependency
    --as-explicit           Mark as explicitly installed
    --no-optional           Skip optional dependencies

EXAMPLES:
    xpm install firefox
    xpm install vim neovim tmux
    xpm S -w linux linux-headers
    xpm install --as-deps libfoo
"#
        ),
        "remove" | "R" => println!(
            r#"xpm remove — Remove Packages

USAGE:
    xpm remove <PACKAGES>... [OPTIONS]
    xpm R <PACKAGES>... [OPTIONS]

DESCRIPTION:
    Remove installed packages from the system.

ARGUMENTS:
    <PACKAGES>      One or more package names to remove

OPTIONS:
    -s, --recursive     Also remove unneeded dependencies
    -d, --no-deps       Skip dependency checking
    -n, --nosave        Remove configuration files (purge)

EXAMPLES:
    xpm remove firefox
    xpm R -s vim           # Remove with unused deps
    xpm remove -n --recursive pkg
"#
        ),
        "upgrade" | "Su" => println!(
            r#"xpm upgrade — System Upgrade

USAGE:
    xpm upgrade [OPTIONS]
    xpm Su [OPTIONS]

DESCRIPTION:
    Upgrade all installed packages to their latest available versions.
    Run `xpm sync` first to get the latest database.

OPTIONS:
    --force             Force reinstall of up-to-date packages
    --ignore <PKG>      Skip specific packages (repeatable)

EXAMPLES:
    xpm upgrade
    xpm Su --ignore linux
    xpm upgrade --ignore pkg1 --ignore pkg2
"#
        ),
        "query" | "Q" => println!(
            r#"xpm query — Query Local Database

USAGE:
    xpm query [FILTER] [OPTIONS]
    xpm Q [FILTER] [OPTIONS]

DESCRIPTION:
    Query the local package database for installed packages.

ARGUMENTS:
    [FILTER]        Optional package name filter

OPTIONS:
    -e, --explicit      List only explicitly installed packages
    -d, --deps          List only packages installed as dependencies
    -t, --orphans       List orphan packages (no longer required)
    -u, --upgrades      List packages with available updates

EXAMPLES:
    xpm query               # List all installed
    xpm Q -e                # Explicit packages only
    xpm query --orphans     # Find orphans
    xpm Q -u                # List upgradeable
"#
        ),
        "search" | "Ss" => println!(
            r#"xpm search — Search Packages

USAGE:
    xpm search <QUERY> [OPTIONS]
    xpm Ss <QUERY> [OPTIONS]

DESCRIPTION:
    Search for packages in the synchronized databases by name,
    description, or provides.

ARGUMENTS:
    <QUERY>         Search term

OPTIONS:
    -l, --local     Search in local database instead of sync

EXAMPLES:
    xpm search firefox
    xpm Ss "text editor"
    xpm search --local vim
"#
        ),
        "info" | "Si" | "Qi" => println!(
            r#"xpm info — Package Information

USAGE:
    xpm info <PACKAGE> [OPTIONS]
    xpm Si <PACKAGE> [OPTIONS]

DESCRIPTION:
    Display detailed information about a package including version,
    description, dependencies, and more.

ARGUMENTS:
    <PACKAGE>       Package name to inspect

OPTIONS:
    -l, --local     Query local database instead of sync

EXAMPLES:
    xpm info linux
    xpm Si firefox
    xpm info --local vim
"#
        ),
        "files" | "Ql" => println!(
            r#"xpm files — List Package Files

USAGE:
    xpm files <PACKAGE>
    xpm Ql <PACKAGE>

DESCRIPTION:
    List all files owned by an installed package.

ARGUMENTS:
    <PACKAGE>       Package name

EXAMPLES:
    xpm files bash
    xpm Ql linux
"#
        ),
        "repo" => println!(
            r#"xpm repo — Repository Management

USAGE:
    xpm repo <ACTION>

ACTIONS:
    list                    List all active repositories
    add <name> <url>        Add a user repository
    remove <name>           Remove a user repository

EXAMPLES:
    xpm repo list
    xpm repo add chaotic-aur https://cdn-mirror.chaotic.cx/$repo/$arch
    xpm repo remove chaotic-aur

See `xpm help repos` for more details on repository configuration.
"#
        ),
        "history" => println!(
            r#"xpm history — Transaction Journal

USAGE:
    xpm history [--json]

DESCRIPTION:
    Show past xpm transactions, newest first. When generations are
    available, each entry shows the generation it produced (`gen:NNNN`).

OPTIONS:
    --json          One JSON object per transaction (machine output)

EXAMPLES:
    xpm history
    xpm history --json
"#
        ),
        "rollback" => println!(
            r#"xpm rollback — Undo the Last Transaction

USAGE:
    xpm rollback [--last | --journal <ID>] [--dry-run]

DESCRIPTION:
    Compute the inverse of a successful transaction and replay it using
    the local package cache. This is the package-level recovery path;
    whole-system recovery remains `x gen rollback`.

OPTIONS:
    --last              Undo the newest successful transaction (default)
    --journal <ID>      Undo a specific journal id (see `xpm history`)
    --dry-run           Print the inverse plan without changing anything

EXAMPLES:
    xpm rollback --dry-run
    xpm rollback --last
    xpm rollback --journal 1727900000-1234
"#
        ),
        "diff" => println!(
            r#"xpm diff — Compare Against a Generation

USAGE:
    xpm diff <GENERATION> [--json]

DESCRIPTION:
    Compare the installed packages (local database) against the
    `packages.tsv` capture of a generation. Use `current` for the
    default generation.

OPTIONS:
    --json              Emit a single JSON object

EXAMPLES:
    xpm diff 0003
    xpm diff current --json
"#
        ),
        _ => println!(
            "Unknown command or topic: {cmd}\n\n\
             Use `xpm help commands` to see all available commands.\n\
             Use `xpm help` for general help."
        ),
    }
}
