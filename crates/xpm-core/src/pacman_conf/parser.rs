//! Line-oriented pacman.conf parser.

use super::*;

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

pub(super) fn under_root(root: &Path, absolute: &str) -> PathBuf {
    root.join(absolute.trim_start_matches('/'))
}

#[derive(Debug, Clone, Copy)]
enum CurrentSection {
    Options,
    Repo(usize),
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
