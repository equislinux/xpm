//! `[options]` directive handlers.

use super::*;

impl Parser {
    pub(super) fn handle_option(
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

    pub(super) fn handle_flag_option(&mut self, key: &str, file: &Path, line: usize) {
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
}
