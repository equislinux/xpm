//! Source package builds via xpm-makepkg.

use super::*;

pub(super) fn cmd_build(config: &XpmConfig, args: &cli::BuildArgs, no_confirm: bool) -> Result<()> {
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
        preinstall: args.preinstall.clone(),
        chroot_install: args.chroot_install.clone(),
        chroot_nodeps: args.nodeps,
        chroot_refresh: args.chroot_refresh,
        chroot_ttl_days: args.chroot_ttl,
        cache_dir: Some(config.options.cache_dir.clone()),
        xpm_db_path: Some(config.options.db_path.clone()),
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
            ignore_groups: Vec::new(),
            assume_installed: Vec::new(),
        };
        cmd_install(config, &install_args, no_confirm)?;
    }

    Ok(())
}
