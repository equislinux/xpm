//! Mark and deptest commands.

use super::*;
use xpm_core::query;

pub(super) fn cmd_mark(config: &XpmConfig, args: &cli::MarkArgs) -> Result<()> {
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

pub(super) fn cmd_deptest(config: &XpmConfig, args: &cli::DeptestArgs) -> Result<()> {
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
