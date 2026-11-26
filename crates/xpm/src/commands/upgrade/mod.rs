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

mod execute;
mod plan;

pub(super) struct UpgradePlan {
    items: Vec<UpgradePlanItem>,
    removed_upgrade_names: Vec<String>,
    assumed: Vec<PackageCandidate>,
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

    let Some(plan) = plan::build(config, args, no_confirm)? else {
        return Ok(());
    };
    if args.print {
        return Ok(());
    }

    confirm_action(":: Proceed with upgrade? [y/N] ", no_confirm)?;
    execute::run(config, args, plan)
}
