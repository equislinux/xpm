//! Bootstrap an empty root using only xpm.

use super::*;

/// Copies only public trust material from the host keyring into a bootstrap
/// root. Private key directories are never copied.
pub(super) fn copy_public_keyring(host_gpg: &Path, target_gpg: &Path) -> Result<()> {
    if !host_gpg.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(target_gpg)?;
    for name in [
        "pubring.gpg",
        "pubring.kbx",
        "trustdb.gpg",
        "trustedkeys.gpg",
        "gpg.conf",
        "common.conf",
    ] {
        let source = host_gpg.join(name);
        if source.is_file() {
            fs::copy(&source, target_gpg.join(name))
                .with_context(|| format!("failed to copy keyring file {}", source.display()))?;
        }
    }
    Ok(())
}

pub(super) fn bootstrap_config(config: &XpmConfig, root: &Path, host_gpg_dir: &Path) -> XpmConfig {
    let mut boot = config.clone();
    boot.options.root_dir = root.to_path_buf();
    // The target is empty: syncing and installing must use the host keyring.
    // The public part is copied into the target at the end of the bootstrap.
    boot.options.gpg_dir = host_gpg_dir.to_path_buf();
    boot.options.db_path = root.join("var/lib/xpm");
    boot.options.cache_dir = root.join("var/cache/xpm/pkg");
    boot.options.log_file = root.join("var/log/xpm.log");
    boot.options.alpm_hooks = false;
    boot.options.shell_integration = false;
    boot
}

pub(super) fn cmd_bootstrap(
    config: &XpmConfig,
    args: &cli::BootstrapArgs,
    host_gpg_dir: &Path,
) -> Result<()> {
    let root = config.options.root_dir.clone();
    if root == Path::new("/") {
        anyhow::bail!("xpm bootstrap: pass an empty target with --root DIR");
    }
    fs::create_dir_all(root.join("var/lib/xpm"))
        .with_context(|| format!("failed to create {}", root.join("var/lib/xpm").display()))?;

    let mut boot = bootstrap_config(config, &root, host_gpg_dir);

    println!(
        ":: Bootstrapping {} with xpm (no pacman/pacstrap)",
        root.display()
    );
    cmd_sync(&boot, &cli::SyncArgs { force: true })?;

    let base = cli::InstallArgs {
        packages: vec!["filesystem".to_string(), "bash".to_string()],
        download_only: false,
        as_deps: false,
        as_explicit: false,
        no_optional: false,
        needed: false,
        print: false,
        overwrite: Vec::new(),
        ignore_groups: Vec::new(),
        assume_installed: Vec::new(),
    };
    boot.options.scriptlet_mode = xpm_core::hooks::ScriptletMode::Skip;
    cmd_install(&boot, &base, true)?;

    let packages = if args.packages.is_empty() {
        vec!["base-devel".to_string()]
    } else {
        args.packages.clone()
    };
    let install = cli::InstallArgs {
        packages,
        download_only: false,
        as_deps: false,
        as_explicit: false,
        no_optional: false,
        needed: false,
        print: false,
        overwrite: Vec::new(),
        ignore_groups: Vec::new(),
        assume_installed: Vec::new(),
    };
    boot.options.scriptlet_mode = xpm_core::hooks::ScriptletMode::Auto;
    cmd_install(&boot, &install, true)?;

    // ALPM hooks are disabled while bootstrapping (some of them call pacman).
    // Run the essential finalizers explicitly: ca-certificates is required by
    // curl/makepkg inside the build chroot, and ldconfig/sysusers/tmpfiles keep
    // the template usable.
    let finalize = "if command -v update-ca-trust >/dev/null 2>&1; then update-ca-trust; fi; if command -v ldconfig >/dev/null 2>&1; then ldconfig; fi; if command -v systemd-sysusers >/dev/null 2>&1; then systemd-sysusers; fi; if command -v systemd-tmpfiles >/dev/null 2>&1; then systemd-tmpfiles --create; fi";
    let status = std::process::Command::new("arch-chroot")
        .arg(&root)
        .args(["sh", "-c", finalize])
        .status()
        .with_context(|| format!("failed to finalize bootstrap root {}", root.display()))?;
    if !status.success() {
        anyhow::bail!("xpm bootstrap finalization failed with status {status}");
    }

    // Copy only the public/trust material from the effective host keyring.
    // The chroot's xpm needs it for official build dependencies; private keys
    // are deliberately never copied. The target always uses the native xpm
    // keyring path, even when compatibility mode read the host pacman keyring.
    let host_gpg = boot.options.effective_gpg_dir();
    copy_public_keyring(&host_gpg, &root.join("etc/xpm/gnupg"))?;

    // Make the new root self-contained for xpm itself: tools inside the
    // chroot (including the makepkg pacman shim) must not fall back to a
    // repo-less /etc/pacman.conf.
    let mut conf = String::from("[options]\n");
    conf.push_str("root_dir = \"/\"\n");
    conf.push_str("db_path = \"/var/lib/xpm\"\n");
    conf.push_str("cache_dir = \"/var/cache/xpm/pkg\"\n");
    conf.push_str("log_file = \"/var/log/xpm.log\"\n");
    conf.push_str("gpg_dir = \"/etc/xpm/gnupg\"\n");
    conf.push_str("shell_integration = false\n");
    conf.push_str(&format!(
        "parallel_downloads = {}\n",
        boot.options.parallel_downloads.max(1)
    ));
    conf.push_str("sig_level = \"optional\"\n");
    for repo in &boot.repositories {
        conf.push_str("\n[[repo]]\n");
        conf.push_str(&format!("name = \"{}\"\n", repo.name));
        conf.push_str("server = [\n");
        for server in &repo.server {
            conf.push_str(&format!("  \"{}\",\n", server));
        }
        conf.push_str("]\n");
    }
    let conf_path = root.join("etc/xpm.conf");
    if let Some(parent) = conf_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&conf_path, conf)
        .with_context(|| format!("failed to write {}", conf_path.display()))?;

    println!(":: Bootstrap complete: {}", root.display());
    Ok(())
}

#[cfg(test)]
mod tests;
