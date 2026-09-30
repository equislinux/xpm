//! xpm — Modern package manager for X Distribution
//!
//! Entry point for the xpm binary. Handles CLI parsing, configuration loading,
//! logging initialization, and dispatching to the appropriate subcommand handler.

mod cli;

use anyhow::{Context, Result};
use clap::Parser;
use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use tracing::Level;
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command};
use xpm_core::config::Repository;
use xpm_core::install_reason::{retain_by_reason, InstallReason};
use xpm_core::journal::{Journal, JournalPackage};
use xpm_core::repo::RepoManager;
use xpm_core::repo_db::{merge_files_db, parse_sync_db};
use xpm_core::repo_sync::{
    download_first_available, package_download_candidates, sync_repo_databases,
    verify_remote_signature, verify_sha256,
};
use xpm_core::resolver::Version;
use xpm_core::txhooks::run_transaction_hooks;
use xpm_core::{HookChain, Transaction};
use xpm_core::{XpmConfig, XpmError};

fn main() -> Result<()> {
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
        config.options.gpg_dir.join("trustedkeys.gpg"),
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
    println!(":: Sync complete (stub).");
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

fn cmd_install(config: &XpmConfig, args: &cli::InstallArgs, no_confirm: bool) -> Result<()> {
    if args.as_deps && args.as_explicit {
        anyhow::bail!("xpm install: --as-deps and --as-explicit are mutually exclusive");
    }
    let reason = if args.as_deps {
        InstallReason::Dep
    } else {
        InstallReason::Explicit
    };

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

    // Create transaction
    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;

    // Setup hooks chain
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(config.options.root_dir != Path::new("/"));

    let mut journal_pkgs = Vec::new();

    // Phase 1: Download and validate packages
    for pkg_name in &args.packages {
        let mut resolved = None;

        for repo in &config.repositories {
            let db_path = sync_dir.join(format!("{}.db", repo.name));
            if !db_path.exists() {
                continue;
            }

            let db = parse_sync_db(&db_path, &repo.name).with_context(|| {
                format!("failed to parse sync db {}", display_rel_or_abs(&db_path))
            })?;

            if let Some(entry) = db.entries.into_iter().find(|e| e.name == *pkg_name) {
                resolved = Some((repo.clone(), entry));
                break;
            }
        }

        let Some((repo, entry)) = resolved else {
            return Err(XpmError::PackageNotFound {
                name: pkg_name.clone(),
            }
            .into());
        };

        let filename = entry.filename.clone().ok_or_else(|| {
            XpmError::Database(format!(
                "package '{}' in repo '{}' is missing FILENAME metadata",
                entry.name, repo.name
            ))
        })?;
        let dest = cache_dir.join(&filename);
        let urls = package_download_candidates(&repo, &arch, &entry);
        let mirror = download_first_available(&urls, &dest, 3).with_context(|| {
            format!(
                "failed to download '{}' from repo '{}'",
                pkg_name, repo.name
            )
        })?;

        let sig_level = repo.sig_level.unwrap_or(config.options.sig_level);
        let keyring_path = config.options.gpg_dir.join("trustedkeys.gpg");
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

        println!("   downloaded: {}", dest.display());
        println!("   source: {}", mirror);

        journal_pkgs.push(JournalPackage::install(&entry.name, &entry.version));

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

    for pkg_name in &args.packages {
        reason
            .write(&local_db_dir, pkg_name)
            .with_context(|| format!("failed to record install reason for '{pkg_name}'"))?;
    }

    println!(
        ":: {} package(s) installed successfully.",
        args.packages.len()
    );
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
    for (pkg_name, local_version) in installed {
        if args.ignore.iter().any(|i| i == &pkg_name) {
            continue;
        }

        let Some((repo, entry)) = remote_latest.get(&pkg_name) else {
            continue;
        };

        let ordering = Version::cmp_versions(&entry.version, &local_version);
        if ordering.is_gt() || (args.force && ordering.is_eq()) {
            planned.push((repo.clone(), entry.clone(), local_version));
        }
    }

    if planned.is_empty() {
        println!(":: Nothing to do. System is up to date.");
        return Ok(());
    }

    println!(":: Packages to upgrade: {}", planned.len());
    for (_, entry, local_version) in &planned {
        println!("   {} {} -> {}", entry.name, local_version, entry.version);
    }

    confirm_action(":: Proceed with upgrade? [y/N] ", no_confirm)?;

    let preserved_reasons: Vec<(String, InstallReason)> = planned
        .iter()
        .filter_map(|(_, entry, _)| {
            InstallReason::read_optional(&local_db_dir, &entry.name)
                .map(|reason| (entry.name.clone(), reason))
        })
        .collect();

    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;
    let hooks = HookChain::default();
    tx.set_hooks(hooks);
    tx.set_shell_integration(config.options.root_dir != Path::new("/"));

    let mut journal_pkgs = Vec::new();

    for (repo, entry, local_version) in planned {
        let filename = entry.filename.clone().ok_or_else(|| {
            XpmError::Database(format!(
                "package '{}' in repo '{}' is missing FILENAME metadata",
                entry.name, repo.name
            ))
        })?;

        let dest = cache_dir.join(&filename);
        let urls = package_download_candidates(&repo, &arch, &entry);
        let mirror = download_first_available(&urls, &dest, 3).with_context(|| {
            format!(
                "failed to download '{}' from repo '{}'",
                entry.name, repo.name
            )
        })?;

        let sig_level = repo.sig_level.unwrap_or(config.options.sig_level);
        let keyring_path = config.options.gpg_dir.join("trustedkeys.gpg");
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

        journal_pkgs.push(JournalPackage::upgrade(
            &entry.name,
            &local_version,
            &entry.version,
        ));

        tx.add_remove(entry.name.clone())
            .context("failed to add remove op to transaction")?;
        tx.add_install(entry.name.clone(), entry.version.clone(), dest)
            .context("failed to add install op to transaction")?;
    }

    commit_transaction(config, "upgrade", journal_pkgs, &mut tx)?;

    for (name, reason) in &preserved_reasons {
        reason
            .write(&local_db_dir, name)
            .with_context(|| format!("failed to preserve install reason for '{name}'"))?;
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
        println!(
            ":: Preparing transaction ({} operation(s))...",
            tx.operation_count()
        );
        tx.prepare().context("transaction preparation failed")?;
        println!(":: Committing transaction...");
        tx.commit().context("transaction commit failed")?;
        run_phase_hooks(config, &journal, "post", action, false)?;
        Ok(())
    })();

    match &result {
        Ok(()) => {
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
        anyhow::bail!(
            "xpm query: --orphans needs dependency tracking (which packages require each \
             installed package), which the local database does not record yet"
        );
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

fn cmd_search(_config: &XpmConfig, args: &cli::SearchArgs) -> Result<()> {
    let db = if args.local { "local" } else { "sync" };
    println!(":: Searching {db} database for '{}'...", args.query);
    println!(":: Search complete (stub).");
    Ok(())
}

fn cmd_info(_config: &XpmConfig, args: &cli::InfoArgs) -> Result<()> {
    let db = if args.local { "local" } else { "sync" };
    println!(":: Package info ({db}): {}", args.package);
    println!(":: Info complete (stub).");
    Ok(())
}

fn cmd_files(_config: &XpmConfig, args: &cli::FilesArgs) -> Result<()> {
    println!(":: Files owned by '{}':", args.package);
    println!(":: File listing complete (stub).");
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
    gpg_dir = "/etc/pacman.d/gnupg/"  # GPG keyring
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
    Dependencies are resolved automatically.

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
        _ => println!(
            "Unknown command or topic: {cmd}\n\n\
             Use `xpm help commands` to see all available commands.\n\
             Use `xpm help` for general help."
        ),
    }
}
