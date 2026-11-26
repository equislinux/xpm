//! Lowering a parsed `pacman.conf` into xpm configuration.

use super::*;

// ── pacman.conf lowering ───────────────────────────────────────────────────

fn sig_check_to_level(check: crate::pacman_conf::SigCheck) -> SigLevel {
    match check {
        crate::pacman_conf::SigCheck::Never => SigLevel::Never,
        crate::pacman_conf::SigCheck::Optional => SigLevel::Optional,
        crate::pacman_conf::SigCheck::Required => SigLevel::Required,
    }
}

impl XpmConfig {
    /// Build an xpm configuration from a parsed pacman.conf.
    ///
    /// xpm keeps its own local database (`/var/lib/xpm`). Repositories, trust
    /// policy, cache, keyring and architecture come from the explicitly
    /// supplied pacman.conf; pacman DB mirroring remains opt-in.
    pub fn from_pacman(pacman: crate::pacman_conf::PacmanConfig) -> Self {
        let global = pacman.options.sig_level;
        let database_level = sig_check_to_level(global.database_check());
        let options = GeneralOptions {
            root_dir: pacman.options.root_dir.clone(),
            db_path: PathBuf::from("/var/lib/xpm/"),
            cache_dir: pacman
                .options
                .cache_dirs
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("/var/cache/xpm/pkg/")),
            log_file: pacman.options.log_file.clone(),
            gpg_dir: pacman.options.gpg_dir.clone(),
            hold_pkg: pacman.options.hold_pkg.clone(),
            ignore_pkg: pacman.options.ignore_pkg.clone(),
            ignore_group: pacman.options.ignore_group.clone(),
            architecture: pacman.options.architectures.first().map(|arch| {
                if arch == "auto" {
                    std::env::consts::ARCH.to_string()
                } else {
                    arch.clone()
                }
            }),
            sig_level: sig_check_to_level(global.package_check()),
            database_sig_level: Some(database_level),
            color: matches!(pacman.options.color, crate::pacman_conf::ColorMode::Always),
            parallel_downloads: pacman.options.parallel_downloads.max(1),
            check_space: pacman.options.check_space,
            alpm_hooks: true,
            shell_integration: false,
            scriptlet_mode: ScriptletMode::Auto,
        };

        let repositories = pacman
            .repositories
            .into_iter()
            .map(|repo| {
                let effective = repo.effective_sig_level(&global);
                Repository {
                    name: repo.name,
                    server: repo.servers,
                    sig_level: Some(sig_check_to_level(effective.package_check())),
                    database_sig_level: Some(sig_check_to_level(effective.database_check())),
                }
            })
            .collect();

        XpmConfig {
            options,
            repositories,
        }
    }
}
