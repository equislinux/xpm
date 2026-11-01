//! Reader for pacman 7.1.0 `pacman.conf` files.
//!
//! The parser mirrors `src/pacman/ini.c` and `src/pacman/conf.c`: line-oriented
//! INI, textual `Include` expansion with shared section state, raw `SigLevel`
//! bitfields, and pacman's option defaults. Paths are never resolved or
//! canonicalized; `$repo` and `$arch` are left untouched for the caller.

use std::fmt;
use std::fs;
use std::ops::{BitOr, BitOrAssign};
use std::path::{Path, PathBuf};

use crate::error::{ConfigError, XpmError, XpmResult};

const MAX_INCLUDE_DEPTH: usize = 10;

const DEFAULT_ROOT_DIR: &str = "/";
const DEFAULT_DB_PATH: &str = "/var/lib/pacman/";
const DEFAULT_CACHE_DIR: &str = "/var/cache/pacman/pkg/";
const DEFAULT_LOG_FILE: &str = "/var/log/pacman.log";
const DEFAULT_GPG_DIR: &str = "/etc/pacman.d/gnupg/";
const DEFAULT_HOOK_DIR: &str = "/etc/pacman.d/hooks/";

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

/// Parses a pacman 7.1.0 configuration file.
pub fn parse_pacman_conf(path: &Path) -> XpmResult<PacmanConfig> {
    let mut parser = Parser::new();
    parser.parse_file(path, 0)?;
    let options = parser.options.finish();

    let repositories = parser
        .repos
        .into_iter()
        .filter(|repo| !repo.duplicate)
        .map(RepoBuilder::finish)
        .collect();

    Ok(PacmanConfig {
        options,
        repositories,
        sources: parser.sources,
        warnings: parser.warnings,
    })
}

fn config_error(file: &Path, line: usize, message: impl fmt::Display) -> XpmError {
    XpmError::Config(ConfigError::Validation {
        message: format!("config file {}, line {}: {}", file.display(), line, message),
    })
}

fn split_values(value: &str) -> impl Iterator<Item = &str> {
    value.split(' ').filter(|token| !token.is_empty())
}

fn under_root(root: &Path, absolute: &str) -> PathBuf {
    root.join(absolute.trim_start_matches('/'))
}

#[derive(Debug, Clone, Copy)]
enum CurrentSection {
    Options,
    Repo(usize),
}

#[derive(Debug)]
struct RepoBuilder {
    name: String,
    servers: Vec<String>,
    cache_servers: Vec<String>,
    sig_level: Option<SigLevelState>,
    usage: Option<Usage>,
    source: SourceLoc,
    duplicate: bool,
}

impl RepoBuilder {
    fn new(name: String, source: SourceLoc, duplicate: bool) -> Self {
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

    fn finish(self) -> PacmanRepo {
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
struct OptionsBuilder {
    root_dir: Option<PathBuf>,
    db_path: Option<PathBuf>,
    cache_dirs: Vec<PathBuf>,
    log_file: Option<PathBuf>,
    gpg_dir: Option<PathBuf>,
    hook_dirs: Vec<PathBuf>,
    architectures: Vec<String>,
    hold_pkg: Vec<String>,
    ignore_pkg: Vec<String>,
    ignore_group: Vec<String>,
    no_upgrade: Vec<String>,
    no_extract: Vec<String>,
    sig_level: SigLevelState,
    local_file_sig_level: Option<SigLevelState>,
    remote_file_sig_level: Option<SigLevelState>,
    check_space: bool,
    parallel_downloads: u32,
    color: Option<ColorMode>,
    use_syslog: bool,
    verbose_pkg_lists: bool,
    clean_method_bits: u8,
    download_user: Option<String>,
    no_progress_bar: bool,
    disable_download_timeout: bool,
    xfer_command: Option<String>,
    ilovecandy: bool,
    disable_sandbox_filesystem: bool,
    disable_sandbox_syscalls: bool,
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
    fn finish(self) -> PacmanOptions {
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

struct Parser {
    options: OptionsBuilder,
    repos: Vec<RepoBuilder>,
    current: Option<CurrentSection>,
    sources: Vec<PathBuf>,
    warnings: Vec<String>,
}

impl Parser {
    fn new() -> Self {
        Self {
            options: OptionsBuilder::default(),
            repos: Vec::new(),
            current: None,
            sources: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn warn(&mut self, file: &Path, line: usize, message: impl fmt::Display) {
        let message = format!("config file {}, line {}: {}", file.display(), line, message);
        tracing::warn!("{}", message);
        self.warnings.push(message);
    }

    fn parse_file(&mut self, path: &Path, depth: usize) -> XpmResult<()> {
        let contents = fs::read_to_string(path)
            .map_err(|source| XpmError::Config(ConfigError::ReadError { source }))?;
        self.sources.push(path.to_path_buf());

        for (index, raw_line) in contents.lines().enumerate() {
            let line_number = index + 1;
            let line = raw_line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(name) = section_name(line) {
                self.begin_section(name, path, line_number);
                continue;
            }

            let (key, value) = match line.split_once('=') {
                Some((key, value)) => (key.trim(), Some(value.trim())),
                None => (line, None),
            };

            if key == "Include" {
                self.handle_include(value, path, line_number, depth)?;
                continue;
            }

            match self.current {
                Some(CurrentSection::Options) => {
                    self.handle_option(key, value, path, line_number)?;
                }
                Some(CurrentSection::Repo(index)) => {
                    self.handle_repo(index, key, value, path, line_number)?;
                }
                None => {
                    return Err(config_error(
                        path,
                        line_number,
                        "All directives must belong to a section.",
                    ));
                }
            }
        }

        Ok(())
    }

    fn begin_section(&mut self, name: &str, file: &Path, line: usize) {
        if name == "options" {
            self.current = Some(CurrentSection::Options);
            return;
        }

        let duplicate = self.repos.iter().any(|repo| repo.name == name);
        let index = self.repos.len();
        self.repos.push(RepoBuilder::new(
            name.to_string(),
            SourceLoc {
                file: file.to_path_buf(),
                line,
            },
            duplicate,
        ));
        self.current = Some(CurrentSection::Repo(index));

        if duplicate {
            self.warn(
                file,
                line,
                format!("repository '{name}' is already defined; ignoring duplicate section"),
            );
        }
    }

    fn handle_include(
        &mut self,
        value: Option<&str>,
        file: &Path,
        line: usize,
        depth: usize,
    ) -> XpmResult<()> {
        let Some(pattern) = value else {
            return Err(config_error(
                file,
                line,
                "directive 'Include' needs a value",
            ));
        };
        if depth >= MAX_INCLUDE_DEPTH {
            return Err(config_error(
                file,
                line,
                format!("config parsing exceeded max recursion depth of {MAX_INCLUDE_DEPTH}"),
            ));
        }

        for include in expand_include(pattern, file, line)? {
            match self.parse_file(&include, depth + 1) {
                Ok(()) => {}
                Err(XpmError::Config(ConfigError::ReadError { source })) => {
                    return Err(config_error(
                        file,
                        line,
                        format!("could not read include '{}': {source}", include.display()),
                    ));
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn handle_option(
        &mut self,
        key: &str,
        value: Option<&str>,
        file: &Path,
        line: usize,
    ) -> XpmResult<()> {
        let Some(value) = value else {
            self.handle_flag_option(key, file, line);
            return Ok(());
        };

        match key {
            "NoUpgrade" => self
                .options
                .no_upgrade
                .extend(split_values(value).map(str::to_owned)),
            "NoExtract" => self
                .options
                .no_extract
                .extend(split_values(value).map(str::to_owned)),
            "IgnorePkg" => self
                .options
                .ignore_pkg
                .extend(split_values(value).map(str::to_owned)),
            "IgnoreGroup" => self
                .options
                .ignore_group
                .extend(split_values(value).map(str::to_owned)),
            "HoldPkg" => self
                .options
                .hold_pkg
                .extend(split_values(value).map(str::to_owned)),
            "CacheDir" => self
                .options
                .cache_dirs
                .extend(split_values(value).map(PathBuf::from)),
            "HookDir" => self
                .options
                .hook_dirs
                .extend(split_values(value).map(PathBuf::from)),
            "Architecture" => self
                .options
                .architectures
                .extend(split_values(value).map(str::to_owned)),
            "DBPath" => {
                if self.options.db_path.is_none() {
                    self.options.db_path = Some(PathBuf::from(value));
                }
            }
            "RootDir" => {
                if self.options.root_dir.is_none() {
                    self.options.root_dir = Some(PathBuf::from(value));
                }
            }
            "GPGDir" => {
                if self.options.gpg_dir.is_none() {
                    self.options.gpg_dir = Some(PathBuf::from(value));
                }
            }
            "LogFile" => {
                if self.options.log_file.is_none() {
                    self.options.log_file = Some(PathBuf::from(value));
                }
            }
            "DownloadUser" => {
                if self.options.download_user.is_none() {
                    self.options.download_user = Some(value.to_string());
                }
            }
            "XferCommand" => self.options.xfer_command = Some(value.to_string()),
            "CleanMethod" => {
                let mut bits = self.options.clean_method_bits;
                for method in split_values(value) {
                    match method {
                        "KeepInstalled" => bits |= 1,
                        "KeepCurrent" => bits |= 2,
                        _ => {
                            return Err(config_error(
                                file,
                                line,
                                format!("invalid value for 'CleanMethod' : '{method}'"),
                            ));
                        }
                    }
                }
                self.options.clean_method_bits = bits;
            }
            "SigLevel" => {
                if let Some(state) =
                    parse_sig_level(value, self.options.sig_level, key, file, line)?
                {
                    self.options.sig_level = state;
                }
            }
            "LocalFileSigLevel" => {
                let initial = self
                    .options
                    .local_file_sig_level
                    .unwrap_or_else(SigLevelState::use_default);
                if let Some(state) = parse_sig_level(value, initial, key, file, line)? {
                    self.options.local_file_sig_level = Some(state);
                }
            }
            "RemoteFileSigLevel" => {
                let initial = self
                    .options
                    .remote_file_sig_level
                    .unwrap_or_else(SigLevelState::use_default);
                if let Some(state) = parse_sig_level(value, initial, key, file, line)? {
                    self.options.remote_file_sig_level = Some(state);
                }
            }
            "ParallelDownloads" => {
                self.options.parallel_downloads = parse_parallel_downloads(value, file, line)?;
            }
            _ => self.warn(
                file,
                line,
                format!("directive '{key}' in section 'options' not recognized."),
            ),
        }

        Ok(())
    }

    fn handle_flag_option(&mut self, key: &str, file: &Path, line: usize) {
        match key {
            "UseSyslog" => self.options.use_syslog = true,
            "ILoveCandy" => self.options.ilovecandy = true,
            "VerbosePkgLists" => self.options.verbose_pkg_lists = true,
            "CheckSpace" => self.options.check_space = true,
            "Color" => {
                if self.options.color.is_none() {
                    self.options.color = Some(ColorMode::Auto);
                }
            }
            "NoProgressBar" => self.options.no_progress_bar = true,
            "DisableDownloadTimeout" => self.options.disable_download_timeout = true,
            "DisableSandbox" => {
                self.options.disable_sandbox_filesystem = true;
                self.options.disable_sandbox_syscalls = true;
            }
            "DisableSandboxFilesystem" => self.options.disable_sandbox_filesystem = true,
            "DisableSandboxSyscalls" => self.options.disable_sandbox_syscalls = true,
            _ => self.warn(
                file,
                line,
                format!("directive '{key}' in section 'options' not recognized."),
            ),
        }
    }

    fn handle_repo(
        &mut self,
        index: usize,
        key: &str,
        value: Option<&str>,
        file: &Path,
        line: usize,
    ) -> XpmResult<()> {
        match key {
            "Server" => {
                let value = required_value(value, key, file, line)?;
                self.repos[index].servers.push(value.to_string());
            }
            "CacheServer" => {
                let value = required_value(value, key, file, line)?;
                self.repos[index].cache_servers.push(value.to_string());
            }
            "SigLevel" => {
                let value = required_value(value, key, file, line)?;
                let initial = self.repos[index]
                    .sig_level
                    .unwrap_or_else(SigLevelState::use_default);
                if let Some(state) = parse_sig_level(value, initial, key, file, line)? {
                    self.repos[index].sig_level = Some(state);
                }
            }
            "Usage" => {
                let value = required_value(value, key, file, line)?;
                let initial = self.repos[index].usage.unwrap_or(Usage::NONE);
                if let Some(usage) = parse_usage(value, initial, file, line)? {
                    self.repos[index].usage = Some(usage);
                }
            }
            _ => {
                let name = self.repos[index].name.clone();
                self.warn(
                    file,
                    line,
                    format!("directive '{key}' in section '{name}' not recognized."),
                );
            }
        }

        Ok(())
    }
}

fn required_value<'a>(
    value: Option<&'a str>,
    key: &str,
    file: &Path,
    line: usize,
) -> XpmResult<&'a str> {
    value.ok_or_else(|| config_error(file, line, format!("directive '{key}' needs a value")))
}

fn section_name(line: &str) -> Option<&str> {
    if line.len() >= 2 && line.starts_with('[') && line.ends_with(']') {
        Some(&line[1..line.len() - 1])
    } else {
        None
    }
}

fn expand_include(pattern: &str, file: &Path, line: usize) -> XpmResult<Vec<PathBuf>> {
    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    let entries = glob::glob_with(pattern, options).map_err(|error| {
        config_error(
            file,
            line,
            format!("invalid include pattern '{pattern}': {error}"),
        )
    })?;

    let mut paths = Vec::new();
    for entry in entries {
        match entry {
            Ok(path) => paths.push(path),
            Err(error) => {
                return Err(config_error(
                    file,
                    line,
                    format!("failed to expand include '{pattern}': {error}"),
                ));
            }
        }
    }

    if paths.is_empty() {
        paths.push(PathBuf::from(pattern));
    }
    paths.sort();
    Ok(paths)
}

fn parse_sig_level(
    value: &str,
    initial: SigLevelState,
    key: &str,
    file: &Path,
    line: usize,
) -> XpmResult<Option<SigLevelState>> {
    let mut level = initial.level;
    let mut mask = initial.mask;
    let mut seen = false;

    for original in split_values(value) {
        seen = true;
        let (token, package, database) = if let Some(token) = original.strip_prefix("Package") {
            (token, true, false)
        } else if let Some(token) = original.strip_prefix("Database") {
            (token, false, true)
        } else {
            (original, true, true)
        };

        match token {
            "Never" => {
                if package {
                    level &= !SigLevelState::PACKAGE;
                    mask |= SigLevelState::PACKAGE;
                }
                if database {
                    level &= !SigLevelState::DATABASE;
                    mask |= SigLevelState::DATABASE;
                }
            }
            "Optional" => {
                if package {
                    level |= SigLevelState::PACKAGE | SigLevelState::PACKAGE_OPTIONAL;
                    mask |= SigLevelState::PACKAGE | SigLevelState::PACKAGE_OPTIONAL;
                }
                if database {
                    level |= SigLevelState::DATABASE | SigLevelState::DATABASE_OPTIONAL;
                    mask |= SigLevelState::DATABASE | SigLevelState::DATABASE_OPTIONAL;
                }
            }
            "Required" => {
                if package {
                    level |= SigLevelState::PACKAGE;
                    level &= !SigLevelState::PACKAGE_OPTIONAL;
                    mask |= SigLevelState::PACKAGE | SigLevelState::PACKAGE_OPTIONAL;
                }
                if database {
                    level |= SigLevelState::DATABASE;
                    level &= !SigLevelState::DATABASE_OPTIONAL;
                    mask |= SigLevelState::DATABASE | SigLevelState::DATABASE_OPTIONAL;
                }
            }
            "TrustedOnly" => {
                if package {
                    level &=
                        !(SigLevelState::PACKAGE_MARGINAL_OK | SigLevelState::PACKAGE_UNKNOWN_OK);
                    mask |= SigLevelState::PACKAGE_MARGINAL_OK | SigLevelState::PACKAGE_UNKNOWN_OK;
                }
                if database {
                    level &=
                        !(SigLevelState::DATABASE_MARGINAL_OK | SigLevelState::DATABASE_UNKNOWN_OK);
                    mask |=
                        SigLevelState::DATABASE_MARGINAL_OK | SigLevelState::DATABASE_UNKNOWN_OK;
                }
            }
            "TrustAll" => {
                if package {
                    level |= SigLevelState::PACKAGE_MARGINAL_OK | SigLevelState::PACKAGE_UNKNOWN_OK;
                    mask |= SigLevelState::PACKAGE_MARGINAL_OK | SigLevelState::PACKAGE_UNKNOWN_OK;
                }
                if database {
                    level |=
                        SigLevelState::DATABASE_MARGINAL_OK | SigLevelState::DATABASE_UNKNOWN_OK;
                    mask |=
                        SigLevelState::DATABASE_MARGINAL_OK | SigLevelState::DATABASE_UNKNOWN_OK;
                }
            }
            _ => {
                return Err(config_error(
                    file,
                    line,
                    format!("invalid value for '{key}' : '{original}'"),
                ));
            }
        }
    }

    if !seen {
        return Ok(None);
    }

    Ok(Some(SigLevelState {
        level: level & !SigLevelState::USE_DEFAULT,
        mask,
    }))
}

fn parse_usage(value: &str, initial: Usage, file: &Path, line: usize) -> XpmResult<Option<Usage>> {
    let mut usage = initial;
    let mut seen = false;

    for token in split_values(value) {
        seen = true;
        match token {
            "Sync" => usage |= Usage::SYNC,
            "Search" => usage |= Usage::SEARCH,
            "Install" => usage |= Usage::INSTALL,
            "Upgrade" => usage |= Usage::UPGRADE,
            "All" => usage |= Usage::ALL,
            _ => {
                return Err(config_error(
                    file,
                    line,
                    format!("'Usage' option '{token}' not recognized"),
                ));
            }
        }
    }

    Ok(seen.then_some(usage))
}

fn parse_parallel_downloads(value: &str, file: &Path, line: usize) -> XpmResult<u32> {
    let number: i64 = value.parse().map_err(|_| {
        config_error(
            file,
            line,
            format!("invalid value for 'ParallelDownloads' : '{value}'"),
        )
    })?;

    if number < 1 {
        return Err(config_error(
            file,
            line,
            format!("value for 'ParallelDownloads' has to be positive : '{value}'"),
        ));
    }
    if number > i64::from(i32::MAX) {
        return Err(config_error(
            file,
            line,
            format!("value for 'ParallelDownloads' is too large : '{value}'"),
        ));
    }

    Ok(number as u32)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::*;

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    fn parse(path: &Path) -> PacmanConfig {
        parse_pacman_conf(path).unwrap()
    }

    fn repo<'a>(config: &'a PacmanConfig, name: &str) -> &'a PacmanRepo {
        config
            .repositories
            .iter()
            .find(|repo| repo.name == name)
            .unwrap()
    }

    #[test]
    fn parses_options_repositories_and_includes() {
        let dir = TempDir::new().unwrap();
        let mirror = write(
            dir.path(),
            "mirrorlist",
            "Server = https://mirror.example/$repo/$arch\n\
             Server = https://backup.example/$repo\n",
        );
        let conf = write(
            dir.path(),
            "pacman.conf",
            &format!(
                "[options]\n\
                 Architecture = x86_64\n\
                 CacheDir = /var/cache/pacman/pkg/\n\
                 Color\n\
                 SigLevel = Required DatabaseOptional\n\
                 ParallelDownloads = 5\n\
                 \n\
                 [x]\n\
                 Server = file:///srv/x/$repo\n\
                 SigLevel = PackageRequired\n\
                 \n\
                 [core]\n\
                 Include = {}\n\
                 Usage = Sync Search\n\
                 \n\
                 [extra]\n\
                 Server = https://extra.example/$repo\n",
                mirror.display()
            ),
        );

        let config = parse(&conf);

        assert_eq!(
            config
                .repositories
                .iter()
                .map(|repo| repo.name.as_str())
                .collect::<Vec<_>>(),
            vec!["x", "core", "extra"]
        );
        assert_eq!(
            config.options.cache_dirs,
            vec![PathBuf::from("/var/cache/pacman/pkg/")]
        );
        assert_eq!(config.options.parallel_downloads, 5);
        assert_eq!(config.options.color, ColorMode::Auto);
        assert_eq!(config.options.architectures, vec!["x86_64"]);
        assert_eq!(config.options.clean_method, CleanMethod::KeepInstalled);
        assert_eq!(config.sources, vec![conf.clone(), mirror.clone()]);
        assert!(config.warnings.is_empty());

        let x = repo(&config, "x");
        assert_eq!(x.servers, vec!["file:///srv/x/$repo"]);
        assert_eq!(x.usage, Usage::ALL);
        assert_eq!(x.source.file, conf);
        assert_eq!(x.source.line, 8);
        assert_eq!(
            x.effective_sig_level(&config.options.sig_level)
                .package_check(),
            SigCheck::Required
        );
        assert_eq!(
            x.effective_sig_level(&config.options.sig_level)
                .database_check(),
            SigCheck::Optional
        );

        let core = repo(&config, "core");
        assert_eq!(
            core.servers,
            vec![
                "https://mirror.example/$repo/$arch",
                "https://backup.example/$repo"
            ]
        );
        assert_eq!(core.usage, Usage::SYNC | Usage::SEARCH);
        assert_eq!(core.sig_level, None);
        assert_eq!(
            core.effective_sig_level(&config.options.sig_level),
            config.options.sig_level
        );

        let extra = repo(&config, "extra");
        assert_eq!(extra.servers, vec!["https://extra.example/$repo"]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn hashtags_are_not_inline_comments() {
        let dir = TempDir::new().unwrap();
        let conf = write(
            dir.path(),
            "pacman.conf",
            "[options]\n\
             RootDir = /root#not-a-comment\n\
             # a real comment\n\
             [core]\n\
             Server = https://example.invalid/A#B\n",
        );

        let config = parse(&conf);

        assert_eq!(
            config.options.root_dir,
            PathBuf::from("/root#not-a-comment")
        );
        assert_eq!(
            repo(&config, "core").servers,
            vec!["https://example.invalid/A#B"]
        );
    }

    #[test]
    fn includes_share_section_state_and_recurse() {
        let dir = TempDir::new().unwrap();
        let mirror = write(
            dir.path(),
            "mirrorlist",
            "Server = https://one.example/$repo\n",
        );
        let middle = write(
            dir.path(),
            "middle",
            &format!(
                "Include = {}\nCacheServer = https://cache.example/$repo\n",
                mirror.display()
            ),
        );
        let conf = write(
            dir.path(),
            "pacman.conf",
            &format!("[core]\nInclude = {}\n", middle.display()),
        );

        let config = parse(&conf);
        let core = repo(&config, "core");

        assert_eq!(core.servers, vec!["https://one.example/$repo"]);
        assert_eq!(core.cache_servers, vec!["https://cache.example/$repo"]);
        assert_eq!(config.sources, vec![conf, middle, mirror]);
    }

    #[test]
    fn included_section_changes_parent_state() {
        let dir = TempDir::new().unwrap();
        let extra = write(
            dir.path(),
            "extra.conf",
            "[extra]\nServer = https://inner.example/$repo\n",
        );
        let conf = write(
            dir.path(),
            "pacman.conf",
            &format!(
                "[core]\nInclude = {}\nServer = https://outer.example/$repo\n",
                extra.display()
            ),
        );

        let config = parse(&conf);

        assert!(repo(&config, "core").servers.is_empty());
        assert_eq!(
            repo(&config, "extra").servers,
            vec!["https://inner.example/$repo", "https://outer.example/$repo"]
        );
    }

    #[test]
    fn siglevel_matrix_and_mask_merge() {
        let dir = TempDir::new().unwrap();
        let conf = write(
            dir.path(),
            "pacman.conf",
            "[options]\n\
             SigLevel = Required DatabaseOptional\n\
             [plain]\n\
             Server = https://plain.example/$repo\n\
             [trustall]\n\
             Server = https://trust.example/$repo\n\
             SigLevel = PackageTrustAll\n\
             [dbnever]\n\
             Server = https://db.example/$repo\n\
             SigLevel = DatabaseNever\n\
             [pkgoptional]\n\
             Server = https://opt.example/$repo\n\
             SigLevel = PackageOptional\n",
        );

        let config = parse(&conf);
        let global = config.options.sig_level;

        assert_eq!(global.package_check(), SigCheck::Required);
        assert_eq!(global.database_check(), SigCheck::Optional);
        assert_eq!(global.package_trust(), SigTrust::TrustedOnly);
        assert_eq!(global.database_trust(), SigTrust::TrustedOnly);

        let plain = repo(&config, "plain").effective_sig_level(&global);
        assert_eq!(plain, global);

        let trustall = repo(&config, "trustall").effective_sig_level(&global);
        assert_eq!(trustall.package_check(), SigCheck::Required);
        assert_eq!(trustall.database_check(), SigCheck::Optional);
        assert_eq!(trustall.package_trust(), SigTrust::TrustAll);
        assert_eq!(trustall.database_trust(), SigTrust::TrustedOnly);
        assert!(trustall.package_allows_marginal());
        assert!(trustall.package_allows_unknown());
        assert!(!trustall.database_allows_marginal());
        assert!(!trustall.database_allows_unknown());

        let dbnever = repo(&config, "dbnever").effective_sig_level(&global);
        assert_eq!(dbnever.package_check(), SigCheck::Required);
        assert_eq!(dbnever.database_check(), SigCheck::Never);
        assert_eq!(dbnever.database_trust(), SigTrust::TrustedOnly);

        let pkgoptional = repo(&config, "pkgoptional").effective_sig_level(&global);
        assert_eq!(pkgoptional.package_check(), SigCheck::Optional);
        assert_eq!(pkgoptional.database_check(), SigCheck::Optional);
    }

    #[test]
    fn siglevel_tokens_apply_left_to_right() {
        let dir = TempDir::new().unwrap();
        let conf = write(
            dir.path(),
            "pacman.conf",
            "[options]\n\
             SigLevel = Required Never\n\
             [a]\n\
             SigLevel = Never Required DatabaseOptional\n\
             [b]\n\
             SigLevel = PackageTrustAll DatabaseNever\n\
             [c]\n\
             SigLevel = DatabaseOptional PackageRequired\n",
        );

        let config = parse(&conf);
        assert_eq!(config.options.sig_level.package_check(), SigCheck::Never);
        assert_eq!(config.options.sig_level.database_check(), SigCheck::Never);

        let a = repo(&config, "a").effective_sig_level(&config.options.sig_level);
        assert_eq!(a.package_check(), SigCheck::Required);
        assert_eq!(a.database_check(), SigCheck::Optional);

        let b = repo(&config, "b").effective_sig_level(&config.options.sig_level);
        assert_eq!(b.package_check(), SigCheck::Never);
        assert_eq!(b.package_trust(), SigTrust::TrustAll);
        assert_eq!(b.database_check(), SigCheck::Never);

        let c = repo(&config, "c").effective_sig_level(&config.options.sig_level);
        assert_eq!(c.package_check(), SigCheck::Required);
        assert_eq!(c.database_check(), SigCheck::Optional);
    }

    #[test]
    fn architecture_lists_and_auto_are_literal() {
        let dir = TempDir::new().unwrap();
        let conf = write(
            dir.path(),
            "pacman.conf",
            "[options]\n\
             Architecture = x86_64\n\
             Architecture = auto aarch64\n",
        );

        let config = parse(&conf);

        assert_eq!(
            config.options.architectures,
            vec!["x86_64", "auto", "aarch64"]
        );
    }

    #[test]
    fn pattern_lists_append_and_preserve_inversion() {
        let dir = TempDir::new().unwrap();
        let conf = write(
            dir.path(),
            "pacman.conf",
            "[options]\n\
             IgnorePkg = linux linux-lts\n\
             IgnorePkg = foo\n\
             NoUpgrade = etc/* !etc/foo.conf\n\
             NoUpgrade = boot/*\n\
             NoExtract = !usr/share/*\n\
             HoldPkg = pacman xpm\n\
             IgnoreGroup = xfce\n",
        );

        let config = parse(&conf);

        assert_eq!(config.options.ignore_pkg, vec!["linux", "linux-lts", "foo"]);
        assert_eq!(
            config.options.no_upgrade,
            vec!["etc/*", "!etc/foo.conf", "boot/*"]
        );
        assert_eq!(config.options.no_extract, vec!["!usr/share/*"]);
        assert_eq!(config.options.hold_pkg, vec!["pacman", "xpm"]);
        assert_eq!(config.options.ignore_group, vec!["xfce"]);
    }

    #[test]
    fn duplicate_repositories_are_first_wins_with_warning() {
        let dir = TempDir::new().unwrap();
        let conf = write(
            dir.path(),
            "pacman.conf",
            "[core]\n\
             Server = https://one.example/$repo\n\
             [extra]\n\
             Server = https://extra.example/$repo\n\
             [core]\n\
             Server = https://two.example/$repo\n",
        );

        let config = parse(&conf);

        assert_eq!(
            config
                .repositories
                .iter()
                .map(|repo| repo.name.as_str())
                .collect::<Vec<_>>(),
            vec!["core", "extra"]
        );
        assert_eq!(
            repo(&config, "core").servers,
            vec!["https://one.example/$repo"]
        );
        assert_eq!(config.warnings.len(), 1);
        assert!(config.warnings[0].contains("already defined"));
        assert!(config.warnings[0].contains(&format!("{}, line 5", conf.display())));
    }

    #[test]
    fn unknown_directives_warn_and_invalid_values_fail_with_location() {
        let dir = TempDir::new().unwrap();
        let unknown = write(
            dir.path(),
            "unknown.conf",
            "[options]\n\
             Frobnicate = yes\n\
             [core]\n\
             Mystery = value\n",
        );
        let config = parse(&unknown);
        assert_eq!(config.warnings.len(), 2);
        assert!(config.warnings[0].contains("Frobnicate"));
        assert!(config.warnings[0].contains(&format!("{}, line 2", unknown.display())));
        assert!(config.warnings[1].contains("Mystery"));
        assert!(config.warnings[1].contains("core"));

        let bad_sig = write(
            dir.path(),
            "bad-sig.conf",
            "[options]\n\
             SigLevel = PackageNope\n",
        );
        let error = parse_pacman_conf(&bad_sig).unwrap_err().to_string();
        assert!(error.contains(&format!("{}, line 2", bad_sig.display())));
        assert!(error.contains("PackageNope"));

        let bad_parallel = write(
            dir.path(),
            "bad-parallel.conf",
            "[options]\n\
             ParallelDownloads = 0\n",
        );
        let error = parse_pacman_conf(&bad_parallel).unwrap_err().to_string();
        assert!(error.contains(&format!("{}, line 2", bad_parallel.display())));
        assert!(error.contains("ParallelDownloads"));

        let bad_usage = write(
            dir.path(),
            "bad-usage.conf",
            "[options]\n\
             SigLevel = Required\n\
             [core]\n\
             Usage = Search Nope\n",
        );
        let error = parse_pacman_conf(&bad_usage).unwrap_err().to_string();
        assert!(error.contains(&format!("{}, line 4", bad_usage.display())));
        assert!(error.contains("Nope"));
    }

    #[test]
    fn missing_include_is_fatal_and_glob_order_is_sorted() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        let conf = write(
            dir.path(),
            "missing.conf",
            &format!("[core]\nInclude = {}\n", missing.display()),
        );

        let error = parse_pacman_conf(&conf).unwrap_err().to_string();
        assert!(error.contains(&format!("{}, line 2", conf.display())));
        assert!(error.contains("could not read include"));
        assert!(error.contains("does-not-exist"));

        let includes = dir.path().join("conf.d");
        fs::create_dir(&includes).unwrap();
        write(
            &includes,
            "20-second.conf",
            "Server = https://second.example/$repo\n",
        );
        write(
            &includes,
            "10-first.conf",
            "Server = https://first.example/$repo\n",
        );
        let glob_conf = write(
            dir.path(),
            "glob.conf",
            &format!("[core]\nInclude = {}\n", includes.join("*.conf").display()),
        );

        let config = parse(&glob_conf);
        assert_eq!(
            repo(&config, "core").servers,
            vec![
                "https://first.example/$repo",
                "https://second.example/$repo"
            ]
        );
    }

    #[test]
    fn include_depth_is_limited() {
        let dir = TempDir::new().unwrap();
        let mut previous = write(
            dir.path(),
            "leaf.conf",
            "[core]\nServer = https://leaf/$repo\n",
        );
        for index in 0..11 {
            let name = format!("level{index}.conf");
            previous = write(
                dir.path(),
                &name,
                &format!("Include = {}\n", previous.display()),
            );
        }

        let error = parse_pacman_conf(&previous).unwrap_err().to_string();
        assert!(error.contains("max recursion depth"));
    }

    #[test]
    fn server_variables_are_preserved_and_arch_is_validated() {
        let dir = TempDir::new().unwrap();
        let with_arch = write(
            dir.path(),
            "with-arch.conf",
            "[options]\n\
             Architecture = x86_64\n\
             [core]\n\
             Server = https://mirror.example/$repo/$arch\n\
             CacheServer = https://cache.example/$repo/$arch\n",
        );
        let config = parse(&with_arch);
        assert_eq!(
            repo(&config, "core").servers,
            vec!["https://mirror.example/$repo/$arch"]
        );
        assert_eq!(
            repo(&config, "core").cache_servers,
            vec!["https://cache.example/$repo/$arch"]
        );
        assert!(config.validate().is_ok());

        let without_arch = write(
            dir.path(),
            "without-arch.conf",
            "[core]\n\
             Server = https://mirror.example/$repo\n\
             CacheServer = https://cache.example/$arch\n",
        );
        let config = parse(&without_arch);
        assert_eq!(
            repo(&config, "core").servers,
            vec!["https://mirror.example/$repo"]
        );
        let error = config.validate().unwrap_err().to_string();
        assert!(error.contains("$arch"));
        assert!(error.contains("Architecture"));
        assert!(error.contains(&format!("{}, line 1", without_arch.display())));
    }

    #[test]
    fn defaults_root_db_and_options_follow_pacman() {
        let dir = TempDir::new().unwrap();
        let conf = write(
            dir.path(),
            "pacman.conf",
            "[options]\n\
             RootDir = /mnt/root\n\
             CacheDir = /cache/one\n\
             CacheDir = /cache/two\n\
             HookDir = /hooks/one\n",
        );

        let config = parse(&conf);
        let options = &config.options;

        assert_eq!(options.root_dir, PathBuf::from("/mnt/root"));
        assert_eq!(options.db_path, PathBuf::from("/mnt/root/var/lib/pacman/"));
        assert_eq!(
            options.log_file,
            PathBuf::from("/mnt/root/var/log/pacman.log")
        );
        assert_eq!(
            options.cache_dirs,
            vec![PathBuf::from("/cache/one"), PathBuf::from("/cache/two")]
        );
        assert_eq!(options.hook_dirs, vec![PathBuf::from("/hooks/one")]);
        assert_eq!(options.parallel_downloads, 1);
        assert_eq!(options.color, ColorMode::Auto);
        assert_eq!(options.clean_method, CleanMethod::KeepInstalled);
        assert_eq!(options.effective_local_file_sig_level(), options.sig_level);
        assert_eq!(options.effective_remote_file_sig_level(), options.sig_level);
    }

    #[test]
    fn directive_before_section_is_fatal_and_flags_parse() {
        let dir = TempDir::new().unwrap();
        let bad = write(dir.path(), "bad.conf", "RootDir = /\n");
        let error = parse_pacman_conf(&bad).unwrap_err().to_string();
        assert!(error.contains(&format!("{}, line 1", bad.display())));
        assert!(error.contains("must belong to a section"));

        let flags = write(
            dir.path(),
            "flags.conf",
            "[options]\n\
             UseSyslog\n\
             CheckSpace\n\
             VerbosePkgLists\n\
             NoProgressBar\n\
             DisableDownloadTimeout\n\
             ILoveCandy\n\
             DisableSandbox\n\
             XferCommand = /usr/bin/curl %u -o %o\n\
             DownloadUser = alpm\n\
             CleanMethod = KeepCurrent\n",
        );
        let options = parse(&flags).options;
        assert!(options.use_syslog);
        assert!(options.check_space);
        assert!(options.verbose_pkg_lists);
        assert!(options.no_progress_bar);
        assert!(options.disable_download_timeout);
        assert!(options.ilovecandy);
        assert!(options.disable_sandbox_filesystem);
        assert!(options.disable_sandbox_syscalls);
        assert_eq!(
            options.xfer_command.as_deref(),
            Some("/usr/bin/curl %u -o %o")
        );
        assert_eq!(options.download_user.as_deref(), Some("alpm"));
        assert_eq!(options.clean_method, CleanMethod::KeepCurrent);
    }
}
