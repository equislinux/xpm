//! Subcommand implementations and dispatch.
//!
//! The CLI surface lives in [`crate::cli`]. Each command family has its own
//! module here. Shared database readers live in [`installed`], shared
//! transaction orchestration in [`transaction`], and small interactive helpers
//! in [`common`].

mod bootstrap;
mod build;
mod check;
mod common;
mod file_db;
mod help;
mod history;
mod install;
mod installed;
mod misc;
mod query;
mod remove;
mod repo;
mod sync;
mod transaction;
mod upgrade;

use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::cli::{self, Cli, Command};
use xpm_core::alpm_hooks::{self, HookOperation, HookTransaction, HookWhen};
use xpm_core::cache::clean_cache;
use xpm_core::config::{Repository, SigLevel};
use xpm_core::conflict;
use xpm_core::depcheck;
use xpm_core::deptest::{self, InstalledDependency};
use xpm_core::generations;
use xpm_core::groups;
use xpm_core::install_reason::{mark_packages, retain_by_reason, InstallReason};
use xpm_core::journal::{Journal, JournalPackage};
use xpm_core::local_db;
use xpm_core::orphans::{find_orphans, InstalledPackage};
use xpm_core::package::{read_metadata, PackageMeta, PackageMetadata};
use xpm_core::progress::ParallelDownloadProgress;
use xpm_core::progress::{BarFill, OperationProgress};
use xpm_core::replace::matches_ignore_group;
use xpm_core::repo::RepoManager;
use xpm_core::repo_db::{merge_files_db, parse_sync_db_cached, RepoEntry};
use xpm_core::repo_sync::{
    download_first_available, download_first_available_checked, download_first_available_tracked,
    package_download_candidates, sync_repo_databases, verify_remote_signature,
    verify_remote_signature_any, verify_sha256,
};
use xpm_core::resolver::{
    assume_installed_candidates, resolve_closure, resolve_closure_with_installed, DepConstraint,
    PackageCandidate, Requirement, Version,
};
use xpm_core::rollback::{last_rollback_candidate, RollbackOp, RollbackPlan};
use xpm_core::txhooks::run_transaction_hooks;
use xpm_core::{FileLock, HookChain, Transaction};
use xpm_core::{XpmConfig, XpmError, XpmResult};
use xpm_makepkg::{build as build_source_package, BuildOptions};

use bootstrap::cmd_bootstrap;
use build::cmd_build;
use check::cmd_check;
use common::{confirm_action, confirm_replace, display_rel_or_abs, exit_with};
use file_db::cmd_file_db;
use help::cmd_help;
use history::{cmd_diff, cmd_history, cmd_rollback};
use install::cmd_install;
use installed::{
    assume_installed_pkgs, check_transaction_file_conflicts, install_target,
    load_installed_candidates, load_installed_pkgs, read_installed_versions,
    read_latest_remote_entries, resolver_candidate, target_manifest,
};
use misc::{cmd_deptest, cmd_mark};
use query::{cmd_files, cmd_info, cmd_query, cmd_search};
use remove::cmd_remove;
use repo::{cmd_clean, cmd_groups, cmd_import_pacman, cmd_list, cmd_repo};
use sync::{cmd_sync, read_all_sync_entries, sync_repositories};
use transaction::{
    commit_transaction, hook_paths_for_removals, hook_paths_from_targets, journal_dir,
    open_transaction, reconcile_transactions,
};
use upgrade::cmd_upgrade;

/// Routes the parsed CLI command to its implementation.
pub(crate) fn dispatch(cli: &Cli, config: &XpmConfig, host_gpg_dir: &Path) -> Result<()> {
    match &cli.command {
        Command::Sync(args) => cmd_sync(config, args),
        Command::Install(args) => cmd_install(config, args, cli.no_confirm),
        Command::Build(args) => cmd_build(config, args, cli.no_confirm),
        Command::Bootstrap(args) => cmd_bootstrap(config, args, host_gpg_dir),
        Command::Remove(args) => cmd_remove(config, args, cli.no_confirm),
        Command::Upgrade(args) => cmd_upgrade(config, args, cli.no_confirm),
        Command::Query(args) => cmd_query(config, args),
        Command::Search(args) => cmd_search(config, args),
        Command::Info(args) => cmd_info(config, args),
        Command::Files(args) => cmd_files(config, args),
        Command::Repo(args) => cmd_repo(config, args),
        Command::History(args) => cmd_history(config, args),
        Command::Rollback(args) => cmd_rollback(config, args, cli.no_confirm),
        Command::Diff(args) => cmd_diff(config, args),
        Command::ImportPacman(args) => cmd_import_pacman(config, args),
        Command::Check(args) => cmd_check(config, args),
        Command::Mark(args) => cmd_mark(config, args),
        Command::Deptest(args) => cmd_deptest(config, args),
        Command::Groups(args) => cmd_groups(config, args),
        Command::List(args) => cmd_list(config, args),
        Command::Clean(args) => cmd_clean(config, args, cli.no_confirm),
        Command::FileDb(args) => cmd_file_db(config, args),
        Command::Usage(args) => cmd_help(args),
    }
}
