//! Package removal.

use super::*;

pub(super) fn cmd_remove(
    config: &XpmConfig,
    args: &cli::RemoveArgs,
    no_confirm: bool,
) -> Result<()> {
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
    let hooks = HookChain::default();
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
