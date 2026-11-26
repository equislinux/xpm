//! Configuration loading and CLI overrides.
//!
//! `--sysroot` discovers xpm's own configuration inside the guest root.
//! Pacman's configuration is only parsed when the caller passes it with
//! `--config` explicitly.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use xpm_core::config::{DEFAULT_GPG_DIR, PACMAN_GPG_DIR};
use xpm_core::{progress, XpmConfig};

use crate::cli::Cli;

/// Loads the effective configuration and applies every CLI override.
///
/// Returns the configuration together with the host keyring directory, which
/// `bootstrap` needs because the target root is still empty.
pub(crate) fn load(cli: &Cli) -> Result<(XpmConfig, PathBuf)> {
    let sysroot_config = cli.sysroot.as_ref().map(|root| root.join("etc/xpm.conf"));
    let config_path = cli
        .config
        .clone()
        .or(sysroot_config)
        .unwrap_or_else(XpmConfig::default_path);
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
        if pacman_style {
            XpmConfig::from_pacman(
                xpm_core::pacman_conf::parse_pacman_conf(&config_path)
                    .with_context(|| format!("failed to load {}", config_path.display()))?,
            )
        } else {
            XpmConfig::load(&config_path)
                .with_context(|| format!("failed to load config from {}", config_path.display()))?
        }
    } else {
        XpmConfig::load_or_default(&config_path)
            .with_context(|| format!("failed to load config from {}", config_path.display()))?
    };

    // The host keyring must remain reachable for `bootstrap`: the target root
    // is empty, so it cannot verify the signed [x] database until the host
    // public keyring is copied into it.
    let host_gpg_dir = config.options.gpg_dir.clone();

    // Apply CLI overrides. An explicit --root wins over --sysroot.
    let operation_root = cli.root.as_deref().or(cli.sysroot.as_deref());
    config.apply_overrides(
        operation_root,
        cli.dbpath.as_deref(),
        cli.cachedir.as_deref(),
    );

    if cli.no_color {
        config.options.color = false;
    }
    // Accepted for compatibility with older xpm-makepkg builds and scripts.
    let _ = cli.no_pacman_mirror;
    if cli.no_alpm_hooks {
        config.options.alpm_hooks = false;
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

    // Per-phase progress. Disabled by the hidden pacman-compatible flag or
    // XPM_NO_PROGRESS; it never corrupts logs.
    progress::init(!cli.noprogressbar, cli.no_color, cli.verbose > 0);

    if let Some(root) = operation_root {
        if root != Path::new("/") {
            // `--sysroot` always relocates the standard paths into the guest
            // root; `--root` keeps custom config paths unless they are the
            // built-in defaults (preserving the pre-existing behavior).
            let sysroot_active = cli.root.is_none() && cli.sysroot.is_some();
            if cli.dbpath.is_none()
                && (sysroot_active || config.options.db_path == Path::new("/var/lib/xpm/"))
            {
                config.options.db_path = root.join("var/lib/xpm");
            }
            if cli.cachedir.is_none()
                && (sysroot_active || config.options.cache_dir == Path::new("/var/cache/xpm/pkg/"))
            {
                config.options.cache_dir = root.join("var/cache/xpm/pkg");
            }
            if config.options.gpg_dir == Path::new(PACMAN_GPG_DIR) {
                config.options.gpg_dir = root.join("etc/pacman.d/gnupg");
            } else if sysroot_active || config.options.gpg_dir == Path::new(DEFAULT_GPG_DIR) {
                config.options.gpg_dir = root.join("etc/xpm/gnupg");
            }
            if sysroot_active || config.options.log_file == Path::new("/var/log/xpm.log") {
                config.options.log_file = root.join("var/log/xpm.log");
            }
        }
    }

    Ok((config, host_gpg_dir))
}
