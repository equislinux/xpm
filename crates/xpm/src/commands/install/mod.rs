//! Package installation.

use super::*;

mod download;
mod execute;
mod plan;

use download::{prefetch_downloads, verify_signatures_parallel};

pub(super) struct InstallPlan {
    resolved: Vec<(Repository, RepoEntry, bool)>,
    local_metadata: Vec<(PathBuf, Arc<PackageMetadata>)>,
    assumed: Vec<PackageCandidate>,
    arch: String,
}

pub(super) fn cmd_install(
    config: &XpmConfig,
    args: &cli::InstallArgs,
    no_confirm: bool,
) -> Result<()> {
    reconcile_transactions(config)?;
    let Some(plan) = plan::build(config, args)? else {
        return Ok(());
    };
    execute::run(config, args, no_confirm, plan)
}
