//! xpm — Modern package manager for X Distribution
//!
//! Entry point for the xpm binary. Handles CLI parsing, configuration loading,
//! logging initialization, and dispatching to the appropriate subcommand handler.

mod cli;
mod pacman_compat;

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
use xpm_core::check;
use xpm_core::config::Repository;
use xpm_core::conflict;
use xpm_core::depcheck;
use xpm_core::deptest::{self, InstalledDependency};
use xpm_core::generations;
use xpm_core::groups;
use xpm_core::install_reason::{mark_packages, retain_by_reason, InstallReason};
use xpm_core::journal::{Journal, JournalPackage};
use xpm_core::local_db;
use xpm_core::orphans::{find_orphans, InstalledPackage};
use xpm_core::package::{read_metadata, PackageMeta};
use xpm_core::query;
use xpm_core::repo::RepoManager;
use xpm_core::repo_db::{merge_files_db, parse_sync_db, RepoEntry};
use xpm_core::repo_sync::{
    download_first_available, package_download_candidates, sync_repo_databases,
    verify_remote_signature, verify_sha256,
};
use xpm_core::resolver::{
    resolve_closure, resolve_closure_with_installed, DepConstraint, PackageCandidate, Requirement,
    Version,
};
use xpm_core::rollback::{last_rollback_candidate, RollbackOp, RollbackPlan};
use xpm_core::txhooks::run_transaction_hooks;
use xpm_core::{FileLock, HookChain, Transaction};
use xpm_core::{XpmConfig, XpmError, XpmResult};
use xpm_makepkg::{build as build_source_package, BuildOptions};

fn main() -> Result<()> {
    // Rust ignores SIGPIPE by default, so `xpm ... | head` panics with a
    // broken-pipe error; restore the default handler like other CLIs.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let argv = pacman_compat::rewrite_args(std::env::args_os());
    let cli = Cli::parse_from(argv);

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
    let pacman_style = config_path
        .file_name()
        .map(|name| name == "pacman.conf")
        .unwrap_or(false);

    let mut config = if let Some(path) = &cli.config {
        if !path.exists() {
            anyhow::bail!("configuration file not found: {}", path.display());
        }
        if pacman_style {
            XpmConfig::from_pacman(
                xpm_core::pacman_conf::parse_pacman_conf(path)
                    .with_context(|| format!("failed to load {}", path.display()))?,
            )
        } else {
            XpmConfig::load(path)
                .with_context(|| format!("failed to load config from {}", path.display()))?
        }
    } else if config_path.exists() {
        XpmConfig::load(&config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))?
    } else if Path::new("/etc/pacman.conf").exists() {
        XpmConfig::from_pacman(
            xpm_core::pacman_conf::parse_pacman_conf(Path::new("/etc/pacman.conf"))
                .context("failed to load /etc/pacman.conf")?,
        )
    } else {
        XpmConfig::load_or_default(&config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))?
    };

    // Apply CLI overrides
    config.apply_overrides(
        cli.root.as_deref(),
        cli.dbpath.as_deref(),
        cli.cachedir.as_deref(),
    );

    if cli.no_color {
        config.options.color = false;
    }
    if cli.no_pacman_mirror {
        config.options.mirror_pacman = false;
    }
    config.options.shell_integration = cli.shell_integration && !cli.no_shell_integration;
    if let Some(mode) = &cli.scriptlets {
        config.options.scriptlet_mode = match mode.as_str() {
            "skip" => xpm_core::hooks::ScriptletMode::Skip,
            "require" => xpm_core::hooks::ScriptletMode::Require,
            _ => xpm_core::hooks::ScriptletMode::Auto,
        };
    }
    if cli.noscriptlet {
        config.options.scriptlet_mode = xpm_core::hooks::ScriptletMode::Skip;
    }

    if let Some(root) = &cli.root {
        if root != Path::new("/") {
            if cli.dbpath.is_none() && config.options.db_path == Path::new("/var/lib/xpm/") {
                config.options.db_path = root.join("var/lib/xpm");
            }
            if cli.cachedir.is_none()
                && config.options.cache_dir == Path::new("/var/cache/xpm/pkg/")
            {
                config.options.cache_dir = root.join("var/cache/xpm/pkg");
            }
            if config.options.gpg_dir == Path::new("/etc/pacman.d/gnupg/") {
                config.options.gpg_dir = root.join("etc/pacman.d/gnupg");
            }
            if config.options.log_file == Path::new("/var/log/xpm.log") {
                config.options.log_file = root.join("var/log/xpm.log");
            }
            if config.options.pacman_db.is_none() {
                config.options.pacman_db = Some(root.join("var/lib/pacman"));
            }
        }
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
        Command::Build(args) => cmd_build(&config, args, cli.no_confirm),
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
        Command::ImportPacman(args) => cmd_import_pacman(&config, args),
        Command::SyncPacmanDb(args) => cmd_sync_pacman_db(&config, args),
        Command::Check(args) => cmd_check(&config, args),
        Command::Mark(args) => cmd_mark(&config, args),
        Command::Deptest(args) => cmd_deptest(&config, args),
        Command::Groups(args) => cmd_groups(&config, args),
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

    let _lock = FileLock::acquire(&config.options.db_path.join("transaction.lock"))?;
    let mut failures: Vec<(String, String)> = Vec::new();

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
        config
            .options
            .database_sig_level
            .unwrap_or(config.options.sig_level),
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
                failures.push((repo.name.clone(), format!("{err}")));
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
                    failures.push((repo.name.clone(), format!("invalid local database: {err}")));
                }
            }
        } else {
            println!(
                "     local db: not found at {}",
                display_rel_or_abs(&db_path)
            );
        }
    }
    if !failures.is_empty() {
        let summary = failures
            .iter()
            .map(|(repo, error)| format!("{repo}: {error}"))
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("failed to synchronize all databases ({summary})");
    }

    println!(":: Sync complete.");
    Ok(())
}

fn read_all_sync_entries(
    config: &XpmConfig,
    sync_dir: &Path,
) -> Result<Vec<(Repository, RepoEntry)>> {
    let mut entries = Vec::new();
    for repo in &config.repositories {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            continue;
        }
        let db = parse_sync_db(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", display_rel_or_abs(&db_path)))?;
        for entry in db.entries {
            entries.push((repo.clone(), entry));
        }
    }
    Ok(entries)
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
    default_database_sig_level: xpm_core::config::SigLevel,
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
                    default_database_sig_level,
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

fn confirm_replace(old: &str, repo: &str, new: &str, no_confirm: bool) -> Result<bool> {
    if no_confirm || !io::stdin().is_terminal() {
        return Ok(true);
    }

    loop {
        print!("Replace {old} with {repo}/{new}? [Y/n] ");
        io::stdout().flush().context("failed to flush prompt")?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("failed to read replacement answer")?;
        match input.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => continue,
        }
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
struct UpgradePlanItem {
    repo: Repository,
    entry: RepoEntry,
    local_version: String,
    is_new: bool,
    replaced: Option<(String, String, InstallReason)>,
}

struct InstalledRecord {
    name: String,
    version: String,
    repo: String,
    depends: Vec<String>,
    provides: Vec<String>,
    groups: Vec<String>,
    reason: InstallReason,
    pkg_file: PathBuf,
}

fn cmd_build(config: &XpmConfig, args: &cli::BuildArgs, no_confirm: bool) -> Result<()> {
    if args.as_deps && args.as_explicit {
        anyhow::bail!("xpm build: --as-deps and --as-explicit are mutually exclusive");
    }

    let source = if args.path.is_file() {
        args.path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid PKGBUILD path"))?
            .to_path_buf()
    } else {
        args.path.clone()
    };
    let pkgdest = args
        .pkgdest
        .clone()
        .unwrap_or_else(|| source.join(".xpm-packages"));

    let options = BuildOptions {
        pkgdest: pkgdest.clone(),
        srcdest: args.srcdest.clone(),
        builddir: args.builddir.clone(),
        run_checks: !args.nocheck,
        clean: args.clean,
        chroot: args.chroot,
        chroot_dir: args
            .chroot_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("/var/lib/archbuild/xpm-aur")),
        extra_args: args.makepkg_args.clone(),
    };

    println!(":: Building source package in {}", source.display());
    let artifacts = build_source_package(&source, &options)?;
    println!(":: Package base: {}", artifacts.pkgbase);
    for package in &artifacts.packages {
        println!("   {} -> {}", package.name, package.path.display());
    }

    if let Some(path) = &args.json {
        let json =
            serde_json::to_vec_pretty(&artifacts).context("failed to serialize build result")?;
        std::fs::write(path, json)
            .with_context(|| format!("failed to write build result {}", path.display()))?;
    }

    if args.install {
        let install_args = cli::InstallArgs {
            packages: artifacts
                .packages
                .iter()
                .map(|package| package.path.display().to_string())
                .collect(),
            download_only: false,
            as_deps: args.as_deps,
            as_explicit: args.as_explicit,
            no_optional: false,
            needed: false,
            print: false,
            overwrite: Vec::new(),
        };
        cmd_install(config, &install_args, no_confirm)?;
    }

    Ok(())
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
        for name in &package_names {
            if name.contains('/') {
                expanded_names.push((*name).clone());
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
        let installed_candidates = load_installed_candidates(&local_db_dir)?;
        let plan = resolve_closure_with_installed(candidates, installed_candidates, &requirements)
            .with_context(|| {
                format!(
                    "failed to resolve dependencies for: {}",
                    expanded_names.join(", ")
                )
            })?;

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
            ":: Resolved {} package(s): {} explicit, {} as dependencies, {} already satisfied",
            resolved.len(),
            explicit_count,
            resolved.len() - explicit_count,
            already_satisfied
        );
    }

    if args.print {
        for (_, entry, _) in &resolved {
            println!("{} {}", entry.name, entry.version);
        }
        for spec in &local_files {
            println!("{spec}");
        }
        return Ok(());
    }

    if resolved.is_empty() && local_files.is_empty() {
        println!(":: Nothing to do.");
        return Ok(());
    }

    // Create transaction
    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;

    // Setup hooks chain
    let hooks = transaction_hooks(config);
    tx.set_hooks(hooks);
    tx.set_shell_integration(
        config.options.shell_integration && config.options.root_dir != Path::new("/"),
    );
    tx.set_scriptlet_mode(config.options.scriptlet_mode);

    let mut journal_pkgs = Vec::new();
    let mut installed: Vec<InstalledRecord> = Vec::new();
    let overwrite = conflict::OverwritePatterns::parse(&args.overwrite);
    let mut target_manifests: Vec<conflict::TargetManifest> = Vec::new();
    let mut target_pkgs: Vec<depcheck::InstallTarget> = Vec::new();
    let installed_versions = read_installed_versions(&local_db_dir)?;

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
        target_manifests.push(target_manifest(
            &local_db_dir,
            &pkg_name,
            path,
            &metadata.meta.backup,
        )?);
        target_pkgs.push(install_target(&metadata.meta, &version));

        if is_upgrade {
            let old_version = installed_version.unwrap().clone();
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
            version: version.clone(),
            repo: "local".to_string(),
            depends: metadata.meta.depends.clone(),
            provides: metadata.meta.provides.clone(),
            groups: metadata.meta.groups.clone(),
            reason: pkg_reason,
            pkg_file: path.to_path_buf(),
        });
    }

    // Phase 1b: Download and validate the resolved plan (dependencies first)
    for (repo, entry, explicit) in &resolved {
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
        target_manifests.push(target_manifest(
            &local_db_dir,
            &entry.name,
            &dest,
            &metadata.meta.backup,
        )?);
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

        println!("   downloaded: {}", dest.display());
        println!("   source: {}", mirror);

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
            version: entry.version.clone(),
            repo: repo.name.clone(),
            depends: metadata.meta.depends.clone(),
            provides: metadata.meta.provides.clone(),
            groups: metadata.meta.groups.clone(),
            reason: pkg_reason,
            pkg_file: dest.clone(),
        });

        // Add to transaction
        if is_upgrade {
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

    let installed_pkgs = load_installed_pkgs(&local_db_dir)?;
    depcheck::check_install(&installed_pkgs, &target_pkgs)
        .context("installation dependency check failed")?;
    check_transaction_file_conflicts(config, &local_db_dir, &target_manifests, &[], &overwrite)?;

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

    mirror_pacman_records(config, &installed)?;

    println!(":: {} package(s) installed successfully.", installed.len());
    if config.options.shell_integration && config.options.root_dir != Path::new("/") {
        println!(":: Shell integration enabled via ~/.local/bin shims.");
        println!(":: If this shell does not find new commands yet, run: hash -r");
        println!(":: For immediate PATH refresh, run: source ~/.zshrc or source ~/.bashrc");
    }
    Ok(())
}

fn cmd_remove(config: &XpmConfig, args: &cli::RemoveArgs, no_confirm: bool) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let installed = load_installed_pkgs(&local_db_dir)?;
    let options = depcheck::RemoveOptions {
        recursive: args.recursive >= 1,
        recursive_all: args.recursive >= 2,
        cascade: args.cascade,
        unneeded: args.unneeded,
        ignore_versions: args.no_deps == 1,
        skip_deps: args.no_deps >= 2,
    };

    let mut local_groups = groups::GroupIndex::new();
    let package_names: HashSet<String> = installed.iter().map(|pkg| pkg.name.clone()).collect();
    let providers: HashSet<String> = installed
        .iter()
        .flat_map(|pkg| pkg.provides.iter().map(|provide| provide.name.clone()))
        .collect();
    for pkg in &installed {
        local_groups.add(groups::GroupPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            repo: None,
            groups: local_db::read_groups(&local_db_dir, &pkg.name)?,
        });
    }

    let mut expanded_targets = Vec::new();
    for target in &args.packages {
        match local_groups.resolve_target(target, &package_names, &providers) {
            groups::TargetResolution::Group(members) => {
                println!(":: Group {}: {} member(s)", target, members.len());
                for member in members {
                    if !expanded_targets.contains(&member.name) {
                        expanded_targets.push(member.name);
                    }
                }
            }
            _ => expanded_targets.push(target.clone()),
        }
    }

    let plan = depcheck::plan_removal(&installed, &expanded_targets, &options)
        .context("failed to plan removal")?;

    if plan.is_empty() {
        println!(":: Nothing to do.");
        return Ok(());
    }

    println!(":: Removing packages: {}", plan.join(", "));
    if args.recursive >= 1 {
        println!("   (including unneeded dependencies)");
    }

    // Create transaction
    let mut tx = Transaction::new(config.options.root_dir.clone(), local_db_dir.clone())
        .context("failed to create transaction")?;

    // Setup hooks chain
    let hooks = transaction_hooks(config);
    tx.set_hooks(hooks);
    tx.set_shell_integration(
        config.options.shell_integration && config.options.root_dir != Path::new("/"),
    );
    tx.set_scriptlet_mode(config.options.scriptlet_mode);
    tx.set_save_configs(!args.nosave);

    // Add remove operations for each package.
    let mut journal_pkgs = Vec::new();
    for pkg_name in &plan {
        let version = std::fs::read_to_string(local_db_dir.join(pkg_name).join("version"))
            .unwrap_or_default()
            .trim()
            .to_string();
        journal_pkgs.push(JournalPackage::remove(pkg_name, version));

        tx.add_remove(pkg_name.clone())
            .context("failed to add remove to transaction")?;
    }

    confirm_action(":: Proceed with removal? [y/N] ", no_confirm)?;

    // Phase 2/3: prepare, commit, hooks and journal.
    let hook_paths = hook_paths_for_removals(&local_db_dir, &plan)?;
    commit_transaction(config, "remove", journal_pkgs, hook_paths, &mut tx)?;

    println!(":: {} package(s) removed successfully.", plan.len());
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
    let installed_replacements: Vec<xpm_core::replace::InstalledReplacement> = installed
        .iter()
        .map(|(name, version)| xpm_core::replace::InstalledReplacement {
            name: name.clone(),
            version: Version::parse(version),
            reason: InstallReason::read_optional(&local_db_dir, name)
                .unwrap_or(InstallReason::Explicit),
            groups: Vec::new(),
        })
        .collect();
    let mut ignore_patterns = args.ignore.clone();
    ignore_patterns.extend(config.options.ignore_pkg.iter().cloned());
    let found_replacements = xpm_core::replace::find_replacements(
        &installed_replacements,
        &replacement_candidates,
        &ignore_patterns,
        &config.options.ignore_group,
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
    let hooks = transaction_hooks(config);
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

        if let Some(sum) = entry.sha256sum.as_deref() {
            verify_sha256(&dest, sum)?;
        }

        let metadata = read_metadata(&dest)
            .with_context(|| format!("failed to read metadata from {}", dest.display()))?;
        target_manifests.push(target_manifest(
            &local_db_dir,
            &entry.name,
            &dest,
            &metadata.meta.backup,
        )?);
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
            version: entry.version.clone(),
            repo: repo.name.clone(),
            depends: metadata.meta.depends.clone(),
            provides: metadata.meta.provides.clone(),
            groups: metadata.meta.groups.clone(),
            reason: record_reason,
            pkg_file: dest.clone(),
        });
    }

    let installed_pkgs = load_installed_pkgs(&local_db_dir)?;
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

    mirror_pacman_records(config, &installed_records)?;

    println!(":: Upgrade complete.");
    Ok(())
}

fn load_installed_pkgs(local_db_dir: &Path) -> Result<Vec<depcheck::InstalledPkg>> {
    let installed = read_installed_versions(local_db_dir)?;
    let mut packages = Vec::with_capacity(installed.len());

    for (name, version) in installed {
        let depends = local_db::read_depends(local_db_dir, &name)?
            .unwrap_or_default()
            .into_iter()
            .map(|dep| DepConstraint::parse(&dep))
            .collect();
        let provides = local_db::read_provides(local_db_dir, &name)?
            .into_iter()
            .map(|provide| DepConstraint::parse(&provide))
            .collect();
        packages.push(depcheck::InstalledPkg {
            name: name.clone(),
            version: Version::parse(&version),
            depends,
            provides,
            reason: InstallReason::read(local_db_dir, &name),
        });
    }

    Ok(packages)
}

fn load_installed_candidates(local_db_dir: &Path) -> Result<Vec<PackageCandidate>> {
    let installed = read_installed_versions(local_db_dir)?;
    let mut candidates = Vec::with_capacity(installed.len());

    for (name, version) in installed {
        let provides = local_db::read_provides(local_db_dir, &name)?
            .into_iter()
            .map(|provide| DepConstraint::parse(&provide))
            .collect();
        candidates.push(PackageCandidate {
            name,
            version: Version::parse(&version),
            depends: Vec::new(),
            conflicts: Vec::new(),
            provides,
            optdepends: Vec::new(),
        });
    }

    Ok(candidates)
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
fn hook_paths_from_targets(targets: &[conflict::TargetManifest]) -> Vec<(String, HookOperation)> {
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

fn hook_paths_for_removals(
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

fn run_alpm_hooks(
    config: &XpmConfig,
    packages: &[JournalPackage],
    paths: &[(String, HookOperation)],
    when: HookWhen,
) -> Result<()> {
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
fn commit_transaction(
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
    let hooks = transaction_hooks(config);
    tx.set_hooks(hooks);
    tx.set_shell_integration(
        config.options.shell_integration && config.options.root_dir != Path::new("/"),
    );
    tx.set_scriptlet_mode(config.options.scriptlet_mode);

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
                    version: version.clone(),
                    repo: "cache".to_string(),
                    depends: metadata.meta.depends.clone(),
                    provides: metadata.meta.provides.clone(),
                    groups: metadata.meta.groups.clone(),
                    reason,
                    pkg_file: file.clone(),
                });
            }
        }
    }

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

    mirror_pacman_records(config, &reinstall_records)?;

    println!(":: Rollback complete ({} operation(s)).", plan.ops.len());
    Ok(())
}

fn cmd_query(config: &XpmConfig, args: &cli::QueryArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
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
            println!("{owner_path} is owned by {} {}", owner.name, owner.version);
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

fn read_sync_package_names(config: &XpmConfig, sync_dir: &Path) -> Result<HashSet<String>> {
    let mut names = HashSet::new();

    for repo in &config.repositories {
        let db_path = sync_dir.join(format!("{}.db", repo.name));
        if !db_path.exists() {
            continue;
        }
        let db = parse_sync_db(&db_path, &repo.name)
            .with_context(|| format!("failed to parse sync db {}", db_path.display()))?;
        names.extend(db.entries.into_iter().map(|entry| entry.name));
    }

    Ok(names)
}

fn cmd_check(config: &XpmConfig, args: &cli::CheckArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let installed = read_installed_versions(&local_db_dir)?;

    let targets = if args.packages.is_empty() {
        let mut names: Vec<String> = installed.keys().cloned().collect();
        names.sort();
        names
    } else {
        let mut names = Vec::new();
        for name in &args.packages {
            if !installed.contains_key(name) {
                return Err(XpmError::PackageNotFound { name: name.clone() }.into());
            }
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        names
    };

    let mut failed = false;
    for name in &targets {
        match check::check_package(&config.options.root_dir, &local_db_dir, name, args.full) {
            Ok(report) => {
                for path in &report.missing {
                    println!("warning: {name}: {path} (missing)");
                }
                if args.full {
                    for path in &report.altered {
                        println!("warning: {name}: {path} (altered)");
                    }
                    for path in &report.metadata_issues {
                        println!("warning: {name}: {path} (missing metadata)");
                    }
                    if report.metadata_missing {
                        println!("warning: {name}: no stored mtree; full check unavailable");
                    }
                    println!(
                        "{name}: {} total files, {} altered files",
                        report.total,
                        report.altered.len()
                    );
                } else {
                    println!(
                        "{name}: {} total files, {} missing files",
                        report.total,
                        report.missing.len()
                    );
                }
                if !report.is_ok(args.full) {
                    failed = true;
                }
            }
            Err(err) => {
                eprintln!("error: {name}: {err:#}");
                failed = true;
            }
        }
    }

    if failed {
        exit_with(1);
    }
    Ok(())
}

fn cmd_mark(config: &XpmConfig, args: &cli::MarkArgs) -> Result<()> {
    if args.as_deps == args.as_explicit {
        anyhow::bail!("xpm mark: exactly one of --as-deps or --as-explicit is required");
    }

    let reason = if args.as_deps {
        InstallReason::Dep
    } else {
        InstallReason::Explicit
    };
    let local_db_dir = config.options.db_path.join("local");
    let count = mark_packages(&local_db_dir, &args.packages, reason)?;
    let label = if args.as_deps {
        "installed as dependencies"
    } else {
        "explicitly installed"
    };
    println!(":: Marked {count} package(s) as {label}.");
    Ok(())
}

fn cmd_deptest(config: &XpmConfig, args: &cli::DeptestArgs) -> Result<()> {
    let local_db_dir = config.options.db_path.join("local");
    let installed = query::load_local_packages(&local_db_dir)?;
    let candidates: Vec<InstalledDependency> = installed
        .iter()
        .map(query::LocalPackage::dependency_view)
        .collect();

    let unmet = deptest::unmet_dependencies(&args.dependencies, &candidates);
    for spec in &unmet {
        println!("{spec}");
    }

    if !unmet.is_empty() {
        exit_with(127);
    }
    Ok(())
}

fn exit_with(code: i32) -> ! {
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    std::process::exit(code);
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

fn install_target(meta: &PackageMeta, version: &str) -> depcheck::InstallTarget {
    depcheck::InstallTarget {
        name: meta.name.clone(),
        version: Version::parse(version),
        depends: meta
            .depends
            .iter()
            .map(|dep| DepConstraint::parse(dep))
            .collect(),
        provides: meta
            .provides
            .iter()
            .map(|provide| DepConstraint::parse(provide))
            .collect(),
        conflicts: meta
            .conflicts
            .iter()
            .map(|conflict| DepConstraint::parse(conflict))
            .collect(),
    }
}

fn package_file_list(pkg_file: &Path) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for file in xpm_core::package::list_files(pkg_file)
        .with_context(|| format!("failed to list files in {}", pkg_file.display()))?
    {
        let mut path = file.path.to_string_lossy().into_owned();
        while let Some(stripped) = path.strip_prefix("./") {
            path = stripped.to_string();
        }
        if path.is_empty() {
            continue;
        }
        if path.starts_with('/') || path.split('/').any(|component| component == "..") {
            anyhow::bail!(
                "package {} contains an unsafe path: {}",
                pkg_file.display(),
                path
            );
        }
        if file.is_dir && !path.ends_with('/') {
            path.push('/');
        }
        paths.push(path);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn target_manifest(
    local_db_dir: &Path,
    name: &str,
    pkg_file: &Path,
    backups: &[String],
) -> Result<conflict::TargetManifest> {
    let old_files = if local_db_dir.join(name).is_dir() {
        local_db::read_files(local_db_dir, name)?
    } else {
        Vec::new()
    };
    Ok(conflict::TargetManifest {
        name: name.to_string(),
        files: package_file_list(pkg_file)?,
        backups: backups.to_vec(),
        old_files,
    })
}

fn load_installed_manifests(local_db_dir: &Path) -> Result<Vec<conflict::InstalledManifest>> {
    let mut manifests = Vec::new();
    if !local_db_dir.exists() {
        return Ok(manifests);
    }

    for entry in std::fs::read_dir(local_db_dir)
        .with_context(|| format!("failed to read {}", local_db_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        manifests.push(conflict::InstalledManifest {
            files: local_db::read_files(local_db_dir, &name)?,
            backups: local_db::read_backup_list(local_db_dir, &name)?,
            name,
        });
    }
    Ok(manifests)
}

fn check_transaction_file_conflicts(
    config: &XpmConfig,
    local_db_dir: &Path,
    targets: &[conflict::TargetManifest],
    removed: &[String],
    overwrite: &conflict::OverwritePatterns,
) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    let removed: std::collections::HashSet<&str> = removed.iter().map(String::as_str).collect();
    let installed: Vec<conflict::InstalledManifest> = load_installed_manifests(local_db_dir)?
        .into_iter()
        .filter(|manifest| !removed.contains(manifest.name.as_str()))
        .collect();
    let conflicts =
        conflict::check_file_conflicts(&config.options.root_dir, &installed, targets, overwrite)
            .context("failed to check file conflicts")?;

    if conflicts.is_empty() {
        return Ok(());
    }

    eprintln!("error: failed to commit transaction (conflicting files)");
    for file_conflict in &conflicts {
        match file_conflict.kind {
            conflict::ConflictKind::TargetTarget => {
                eprintln!(
                    "{}: {} exists in both '{}' and '{}'",
                    file_conflict.target,
                    file_conflict.path,
                    file_conflict.target,
                    file_conflict.other_target.as_deref().unwrap_or("?")
                );
            }
            conflict::ConflictKind::FilesystemOwned => {
                eprintln!(
                    "{}: {} exists in filesystem (owned by {})",
                    file_conflict.target,
                    file_conflict.path,
                    file_conflict.owner.as_deref().unwrap_or("?")
                );
            }
            conflict::ConflictKind::FilesystemUnowned => {
                eprintln!(
                    "{}: {} exists in filesystem",
                    file_conflict.target, file_conflict.path
                );
            }
        }
    }
    anyhow::bail!("failed to commit transaction (conflicting files)");
}

fn transaction_hooks(config: &XpmConfig) -> HookChain {
    let mut hooks = HookChain::default();
    if let Some(pacman_local) = auto_pacman_local(config) {
        hooks.add_hook(Box::new(xpm_core::hooks::PacmanDbHook::new(
            pacman_local,
            config.options.root_dir.clone(),
        )));
    }
    hooks
}

fn auto_pacman_local(config: &XpmConfig) -> Option<PathBuf> {
    if !config.options.mirror_pacman {
        return None;
    }
    if let Some(db) = &config.options.pacman_db {
        return Some(db.join("local"));
    }
    if config.options.root_dir == Path::new("/") {
        let candidate = PathBuf::from("/var/lib/pacman/local");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

fn mirror_pacman_records(config: &XpmConfig, records: &[InstalledRecord]) -> Result<()> {
    let Some(pacman_local) = auto_pacman_local(config) else {
        return Ok(());
    };
    let xpm_local = config.options.db_path.join("local");
    let mut failures = Vec::new();

    for record in records {
        if let Err(err) = xpm_core::pacman_db::mirror_entry(
            &pacman_local,
            &xpm_local,
            &record.name,
            &record.version,
            record.reason,
            Some(&record.pkg_file),
            &config.options.root_dir,
        ) {
            failures.push(format!("{}: {err:#}", record.name));
        }
    }

    if !failures.is_empty() {
        anyhow::bail!(
            "the xpm transaction succeeded but mirroring into pacman's database failed: {}\nRun `xpm sync-pacman-db` to repair it.",
            failures.join("; ")
        );
    }
    Ok(())
}

fn cmd_groups(config: &XpmConfig, args: &cli::GroupsArgs) -> Result<()> {
    let mut index = groups::GroupIndex::new();

    if args.local {
        let local_db_dir = config.options.db_path.join("local");
        for (name, version) in read_installed_versions(&local_db_dir)? {
            index.add(groups::GroupPackage {
                name: name.clone(),
                version: Version::parse(&version),
                repo: None,
                groups: local_db::read_groups(&local_db_dir, &name)?,
            });
        }
    } else {
        let sync_dir = config.options.db_path.join("sync");
        for repo in &config.repositories {
            let db_path = sync_dir.join(format!("{}.db", repo.name));
            if !db_path.exists() {
                continue;
            }
            let db = parse_sync_db(&db_path, &repo.name).with_context(|| {
                format!("failed to parse sync db {}", display_rel_or_abs(&db_path))
            })?;
            for entry in db.entries {
                index.add(groups::GroupPackage {
                    name: entry.name.clone(),
                    version: Version::parse(&entry.version),
                    repo: Some(repo.name.clone()),
                    groups: entry.groups,
                });
            }
        }
    }

    let pairs = if let Some(group) = &args.group {
        let members = index.group_members_pairs(group);
        if members.is_empty() {
            if args.local {
                anyhow::bail!("group '{group}' was not found");
            }
            anyhow::bail!("package group '{group}' was not found");
        }
        members
    } else {
        index.group_pairs()
    };

    for (group, package) in pairs {
        if args.quiet {
            println!("{package}");
        } else {
            println!("{group} {package}");
        }
    }
    Ok(())
}

fn cmd_sync_pacman_db(config: &XpmConfig, args: &cli::SyncPacmanDbArgs) -> Result<()> {
    let pacman_local = args
        .pacman_db
        .as_ref()
        .map(|db| db.join("local"))
        .or_else(|| auto_pacman_local(config))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "pacman database not found; pass --pacman-db or set `pacman_db` in the config"
            )
        })?;
    let xpm_local = config.options.db_path.join("local");

    let report = xpm_core::pacman_db::sync_all(
        &pacman_local,
        &xpm_local,
        &config.options.cache_dir,
        &config.options.root_dir,
    )
    .context("failed to mirror the xpm database into pacman's")?;

    println!(
        ":: Mirrored {} package(s) into {}, skipped {}.",
        report.written,
        pacman_local.display(),
        report.skipped
    );
    for (package, error) in &report.failed {
        eprintln!("   {package}: {error}");
    }
    if !report.failed.is_empty() {
        anyhow::bail!("{} package(s) could not be mirrored", report.failed.len());
    }
    Ok(())
}

fn cmd_import_pacman(config: &XpmConfig, args: &cli::ImportArgs) -> Result<()> {
    let dest = config.options.db_path.join("local");
    println!(
        ":: Importing pacman local database {} -> {}",
        args.from.display(),
        dest.display()
    );
    let report = xpm_core::migrate::import_pacman_local(&args.from, &dest, args.force)
        .context("failed to import pacman local database")?;

    println!(
        ":: Imported {} package(s), skipped {} already present.",
        report.imported, report.skipped
    );
    for (package, error) in &report.failed {
        eprintln!("   {package}: {error}");
    }
    if !report.failed.is_empty() {
        anyhow::bail!("{} package(s) failed to import", report.failed.len());
    }
    println!(":: Import complete. Run `xpm query` to inspect the local database.");
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
    Qk → check    D → mark       T → deptest

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
    check       Verify installed files against the local database
    deptest     Test dependency specifications against installed packages
    history     Show the transaction journal
    rollback    Undo the last transaction (package level, from the cache)
    diff        Compare installed packages against a generation capture

DATABASE MAINTENANCE:
    mark        Change install reasons without reinstalling

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
    Query the local package database for installed packages. FILTER matches
    the package name exactly; use --search for a substring match.

ARGUMENTS:
    [FILTER]        Exact package name to query

OPTIONS:
    -e, --explicit      List only explicitly installed packages
    -d, --deps          List only packages installed as dependencies
    -t, --orphans       List orphan packages (no longer required)
    -u, --upgrades      List packages with available updates
    -q, --quiet         Print only package names
        --search        Treat FILTER as a substring
        --owner PATH    Show which package owns PATH
    -m, --foreign       List packages absent from all sync databases
        --unrequired    List packages no other package directly requires

EXAMPLES:
    xpm query               # List all installed
    xpm Q -e                # Explicit packages only
    xpm query --orphans     # Find orphans
    xpm query --unrequired --deps  # Unneeded dependency packages
    xpm query --owner /usr/bin/vim
    xpm query --foreign
    xpm Q -u                # List upgradeable
"#
        ),
        "check" | "Qk" => println!(
            r#"xpm check — Verify Installed Files

USAGE:
    xpm check [--full] [PACKAGES]...
    xpm Qk [--full] [PACKAGES]...

DESCRIPTION:
    Verify the files recorded in each installed package's local-database
    manifest. Fast mode checks existence and file/directory type. --full also
    verifies the stored .MTREE SHA-256 digests, symlink targets and modes.

OPTIONS:
    --full          Verify hashes, symlinks and modes (-Qkk)

EXAMPLES:
    xpm check
    xpm check --full
    xpm check hello
"#
        ),
        "mark" | "D" => println!(
            r#"xpm mark — Change Install Reason

USAGE:
    xpm mark --as-deps <PACKAGES>...
    xpm mark --as-explicit <PACKAGES>...

DESCRIPTION:
    Change the recorded install reason without reinstalling the package.

OPTIONS:
    --as-deps       Mark as installed as a dependency
    --as-explicit   Mark as explicitly installed
"#
        ),
        "deptest" | "T" => println!(
            r#"xpm deptest — Test Dependencies

USAGE:
    xpm deptest <DEPENDENCIES>...

DESCRIPTION:
    Test dependency specifications against the installed packages, including
    versioned provides. Unmet specifications are printed one per line and the
    command exits with status 127.

EXAMPLES:
    xpm deptest glibc bash
    xpm deptest 'python>=3.12'
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
