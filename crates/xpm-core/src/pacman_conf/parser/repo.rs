//! Repository section and `Include` handlers.

use super::*;

impl Parser {
    pub(super) fn handle_include(
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

    pub(super) fn handle_repo(
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
