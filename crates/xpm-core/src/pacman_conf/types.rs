//! Public pacman.conf data types.

use super::*;

/// Bitflags for the repositories in which a sync database participates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Usage(u8);

impl Usage {
    pub const NONE: Self = Self(0);
    pub const SYNC: Self = Self(1);
    pub const SEARCH: Self = Self(1 << 1);
    pub const INSTALL: Self = Self(1 << 2);
    pub const UPGRADE: Self = Self(1 << 3);
    pub const ALL: Self = Self(0b1111);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self(bits & Self::ALL.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl Default for Usage {
    fn default() -> Self {
        Self::ALL
    }
}

impl BitOr for Usage {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Usage {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Which old package files `pacman -Sc` (or its xpm equivalent) keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CleanMethod {
    #[default]
    KeepInstalled,
    KeepCurrent,
    KeepInstalledAndCurrent,
}

impl CleanMethod {
    pub const fn keeps_installed(self) -> bool {
        matches!(self, Self::KeepInstalled | Self::KeepInstalledAndCurrent)
    }

    pub const fn keeps_current(self) -> bool {
        matches!(self, Self::KeepCurrent | Self::KeepInstalledAndCurrent)
    }
}

/// Color policy. pacman's bare `Color` directive means "auto" (a tty only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// Signature verification requirement for one object class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigCheck {
    Never,
    Optional,
    Required,
}

/// Signature trust policy for one object class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigTrust {
    TrustedOnly,
    TrustAll,
}

/// Raw libalpm signature level and mask bitfields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigLevelState {
    pub level: u32,
    pub mask: u32,
}

impl SigLevelState {
    pub const PACKAGE: u32 = 1 << 0;
    pub const PACKAGE_OPTIONAL: u32 = 1 << 1;
    pub const PACKAGE_MARGINAL_OK: u32 = 1 << 2;
    pub const PACKAGE_UNKNOWN_OK: u32 = 1 << 3;

    pub const DATABASE: u32 = 1 << 10;
    pub const DATABASE_OPTIONAL: u32 = 1 << 11;
    pub const DATABASE_MARGINAL_OK: u32 = 1 << 12;
    pub const DATABASE_UNKNOWN_OK: u32 = 1 << 13;

    pub const USE_DEFAULT: u32 = 1 << 30;

    const PACKAGE_TRUST: u32 = Self::PACKAGE_MARGINAL_OK | Self::PACKAGE_UNKNOWN_OK;
    const DATABASE_TRUST: u32 = Self::DATABASE_MARGINAL_OK | Self::DATABASE_UNKNOWN_OK;

    pub const ALL_BITS: u32 = 0x0000_f00f;

    pub const fn new(level: u32, mask: u32) -> Self {
        Self { level, mask }
    }

    pub const fn use_default() -> Self {
        Self::new(Self::USE_DEFAULT, 0)
    }

    /// Applies this value over `global` with pacman's `merge_siglevel`
    /// semantics, then resolves a remaining `USE_DEFAULT`.
    pub fn effective(&self, global: &SigLevelState) -> SigLevelState {
        let mut level = if self.mask != 0 {
            (self.level & self.mask) | (global.level & !self.mask)
        } else {
            self.level
        };
        if level & Self::USE_DEFAULT != 0 {
            level = global.level;
        }
        SigLevelState {
            level: level & !Self::USE_DEFAULT,
            mask: Self::ALL_BITS,
        }
    }

    pub fn package_check(&self) -> SigCheck {
        check_for(self.level, Self::PACKAGE, Self::PACKAGE_OPTIONAL)
    }

    pub fn database_check(&self) -> SigCheck {
        check_for(self.level, Self::DATABASE, Self::DATABASE_OPTIONAL)
    }

    pub fn package_trust(&self) -> SigTrust {
        trust_for(self.level, Self::PACKAGE_TRUST)
    }

    pub fn database_trust(&self) -> SigTrust {
        trust_for(self.level, Self::DATABASE_TRUST)
    }

    pub fn package_required(&self) -> bool {
        self.level & Self::PACKAGE != 0
    }

    pub fn package_optional(&self) -> bool {
        self.level & Self::PACKAGE_OPTIONAL != 0
    }

    pub fn database_required(&self) -> bool {
        self.level & Self::DATABASE != 0
    }

    pub fn database_optional(&self) -> bool {
        self.level & Self::DATABASE_OPTIONAL != 0
    }

    pub fn package_allows_marginal(&self) -> bool {
        self.level & Self::PACKAGE_MARGINAL_OK != 0
    }

    pub fn package_allows_unknown(&self) -> bool {
        self.level & Self::PACKAGE_UNKNOWN_OK != 0
    }

    pub fn database_allows_marginal(&self) -> bool {
        self.level & Self::DATABASE_MARGINAL_OK != 0
    }

    pub fn database_allows_unknown(&self) -> bool {
        self.level & Self::DATABASE_UNKNOWN_OK != 0
    }
}

impl Default for SigLevelState {
    fn default() -> Self {
        Self::new(Self::PACKAGE | Self::DATABASE, 0)
    }
}

fn check_for(level: u32, check: u32, optional: u32) -> SigCheck {
    if level & check == 0 {
        SigCheck::Never
    } else if level & optional != 0 {
        SigCheck::Optional
    } else {
        SigCheck::Required
    }
}

fn trust_for(level: u32, trust: u32) -> SigTrust {
    if level & trust == trust {
        SigTrust::TrustAll
    } else {
        SigTrust::TrustedOnly
    }
}

/// Source location of a parsed section or directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLoc {
    pub file: PathBuf,
    pub line: usize,
}

/// One repository section, in first-registration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacmanRepo {
    pub name: String,
    pub servers: Vec<String>,
    pub cache_servers: Vec<String>,
    pub sig_level: Option<SigLevelState>,
    pub usage: Usage,
    pub source: SourceLoc,
}

impl PacmanRepo {
    pub fn effective_sig_level(&self, global: &SigLevelState) -> SigLevelState {
        match self.sig_level {
            Some(state) => state.effective(global),
            None => *global,
        }
    }
}

/// Parsed `[options]` section with pacman's defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacmanOptions {
    pub root_dir: PathBuf,
    pub db_path: PathBuf,
    pub cache_dirs: Vec<PathBuf>,
    pub log_file: PathBuf,
    pub gpg_dir: PathBuf,
    pub hook_dirs: Vec<PathBuf>,
    pub architectures: Vec<String>,
    pub hold_pkg: Vec<String>,
    pub ignore_pkg: Vec<String>,
    pub ignore_group: Vec<String>,
    pub no_upgrade: Vec<String>,
    pub no_extract: Vec<String>,
    pub sig_level: SigLevelState,
    pub local_file_sig_level: Option<SigLevelState>,
    pub remote_file_sig_level: Option<SigLevelState>,
    pub check_space: bool,
    pub parallel_downloads: u32,
    pub color: ColorMode,
    pub use_syslog: bool,
    pub verbose_pkg_lists: bool,
    pub clean_method: CleanMethod,
    pub download_user: Option<String>,
    pub no_progress_bar: bool,
    pub disable_download_timeout: bool,
    pub xfer_command: Option<String>,
    pub ilovecandy: bool,
    pub disable_sandbox_filesystem: bool,
    pub disable_sandbox_syscalls: bool,
}

impl Default for PacmanOptions {
    fn default() -> Self {
        Self {
            root_dir: PathBuf::from(DEFAULT_ROOT_DIR),
            db_path: PathBuf::from(DEFAULT_DB_PATH),
            cache_dirs: vec![PathBuf::from(DEFAULT_CACHE_DIR)],
            log_file: PathBuf::from(DEFAULT_LOG_FILE),
            gpg_dir: PathBuf::from(DEFAULT_GPG_DIR),
            hook_dirs: vec![PathBuf::from(DEFAULT_HOOK_DIR)],
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
            color: ColorMode::Auto,
            use_syslog: false,
            verbose_pkg_lists: false,
            clean_method: CleanMethod::KeepInstalled,
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

impl PacmanOptions {
    pub fn effective_local_file_sig_level(&self) -> SigLevelState {
        match self.local_file_sig_level {
            Some(state) => state.effective(&self.sig_level),
            None => self.sig_level,
        }
    }

    pub fn effective_remote_file_sig_level(&self) -> SigLevelState {
        match self.remote_file_sig_level {
            Some(state) => state.effective(&self.sig_level),
            None => self.sig_level,
        }
    }
}

/// Complete parsed pacman configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacmanConfig {
    pub options: PacmanOptions,
    pub repositories: Vec<PacmanRepo>,
    pub sources: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl PacmanConfig {
    /// Mirrors pacman's post-parse `$arch` check. `$repo` is always valid.
    pub fn validate(&self) -> XpmResult<()> {
        if !self.options.architectures.is_empty() {
            return Ok(());
        }

        for repo in &self.repositories {
            for server in repo.servers.iter().chain(&repo.cache_servers) {
                if server.contains("$arch") {
                    return Err(config_error(
                        &repo.source.file,
                        repo.source.line,
                        format!(
                            "repository '{}' server '{}' uses $arch, but Architecture is not configured",
                            repo.name, server
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}
