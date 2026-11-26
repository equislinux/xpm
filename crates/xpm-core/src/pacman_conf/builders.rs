//! Incremental builders for pacman.conf sections.

use super::*;

#[derive(Debug)]
pub(super) struct RepoBuilder {
    pub(super) name: String,
    pub(super) servers: Vec<String>,
    pub(super) cache_servers: Vec<String>,
    pub(super) sig_level: Option<SigLevelState>,
    pub(super) usage: Option<Usage>,
    pub(super) source: SourceLoc,
    pub(super) duplicate: bool,
}

impl RepoBuilder {
    pub(super) fn new(name: String, source: SourceLoc, duplicate: bool) -> Self {
        Self {
            name,
            servers: Vec::new(),
            cache_servers: Vec::new(),
            sig_level: None,
            usage: None,
            source,
            duplicate,
        }
    }

    pub(super) fn finish(self) -> PacmanRepo {
        PacmanRepo {
            name: self.name,
            servers: self.servers,
            cache_servers: self.cache_servers,
            sig_level: self.sig_level,
            usage: self.usage.unwrap_or(Usage::ALL),
            source: self.source,
        }
    }
}

#[derive(Debug)]
pub(super) struct OptionsBuilder {
    pub(super) root_dir: Option<PathBuf>,
    pub(super) db_path: Option<PathBuf>,
    pub(super) cache_dirs: Vec<PathBuf>,
    pub(super) log_file: Option<PathBuf>,
    pub(super) gpg_dir: Option<PathBuf>,
    pub(super) hook_dirs: Vec<PathBuf>,
    pub(super) architectures: Vec<String>,
    pub(super) hold_pkg: Vec<String>,
    pub(super) ignore_pkg: Vec<String>,
    pub(super) ignore_group: Vec<String>,
    pub(super) no_upgrade: Vec<String>,
    pub(super) no_extract: Vec<String>,
    pub(super) sig_level: SigLevelState,
    pub(super) local_file_sig_level: Option<SigLevelState>,
    pub(super) remote_file_sig_level: Option<SigLevelState>,
    pub(super) check_space: bool,
    pub(super) parallel_downloads: u32,
    pub(super) color: Option<ColorMode>,
    pub(super) use_syslog: bool,
    pub(super) verbose_pkg_lists: bool,
    pub(super) clean_method_bits: u8,
    pub(super) download_user: Option<String>,
    pub(super) no_progress_bar: bool,
    pub(super) disable_download_timeout: bool,
    pub(super) xfer_command: Option<String>,
    pub(super) ilovecandy: bool,
    pub(super) disable_sandbox_filesystem: bool,
    pub(super) disable_sandbox_syscalls: bool,
}

impl Default for OptionsBuilder {
    fn default() -> Self {
        Self {
            root_dir: None,
            db_path: None,
            cache_dirs: Vec::new(),
            log_file: None,
            gpg_dir: None,
            hook_dirs: Vec::new(),
            architectures: Vec::new(),
            hold_pkg: Vec::new(),
            ignore_pkg: Vec::new(),
            ignore_group: Vec::new(),
            no_upgrade: Vec::new(),
            no_extract: Vec::new(),
            sig_level: SigLevelState::default(),
            local_file_sig_level: None,
            remote_file_sig_level: None,
            check_space: false,
            parallel_downloads: 1,
            color: None,
            use_syslog: false,
            verbose_pkg_lists: false,
            clean_method_bits: 0,
            download_user: None,
            no_progress_bar: false,
            disable_download_timeout: false,
            xfer_command: None,
            ilovecandy: false,
            disable_sandbox_filesystem: false,
            disable_sandbox_syscalls: false,
        }
    }
}

impl OptionsBuilder {
    pub(super) fn finish(self) -> PacmanOptions {
        let root_was_set = self.root_dir.is_some();
        let root_dir = self
            .root_dir
            .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT_DIR));

        let db_path = self.db_path.unwrap_or_else(|| {
            if root_was_set {
                under_root(&root_dir, DEFAULT_DB_PATH)
            } else {
                PathBuf::from(DEFAULT_DB_PATH)
            }
        });
        let log_file = self.log_file.unwrap_or_else(|| {
            if root_was_set {
                under_root(&root_dir, DEFAULT_LOG_FILE)
            } else {
                PathBuf::from(DEFAULT_LOG_FILE)
            }
        });

        let cache_dirs = if self.cache_dirs.is_empty() {
            vec![PathBuf::from(DEFAULT_CACHE_DIR)]
        } else {
            self.cache_dirs
        };
        let hook_dirs = if self.hook_dirs.is_empty() {
            vec![PathBuf::from(DEFAULT_HOOK_DIR)]
        } else {
            self.hook_dirs
        };

        PacmanOptions {
            root_dir,
            db_path,
            cache_dirs,
            log_file,
            gpg_dir: self
                .gpg_dir
                .unwrap_or_else(|| PathBuf::from(DEFAULT_GPG_DIR)),
            hook_dirs,
            architectures: self.architectures,
            hold_pkg: self.hold_pkg,
            ignore_pkg: self.ignore_pkg,
            ignore_group: self.ignore_group,
            no_upgrade: self.no_upgrade,
            no_extract: self.no_extract,
            sig_level: self.sig_level,
            local_file_sig_level: self.local_file_sig_level,
            remote_file_sig_level: self.remote_file_sig_level,
            check_space: self.check_space,
            parallel_downloads: self.parallel_downloads,
            color: self.color.unwrap_or(ColorMode::Auto),
            use_syslog: self.use_syslog,
            verbose_pkg_lists: self.verbose_pkg_lists,
            clean_method: match self.clean_method_bits {
                0 | 1 => CleanMethod::KeepInstalled,
                2 => CleanMethod::KeepCurrent,
                _ => CleanMethod::KeepInstalledAndCurrent,
            },
            download_user: self.download_user,
            no_progress_bar: self.no_progress_bar,
            disable_download_timeout: self.disable_download_timeout,
            xfer_command: self.xfer_command,
            ilovecandy: self.ilovecandy,
            disable_sandbox_filesystem: self.disable_sandbox_filesystem,
            disable_sandbox_syscalls: self.disable_sandbox_syscalls,
        }
    }
}
