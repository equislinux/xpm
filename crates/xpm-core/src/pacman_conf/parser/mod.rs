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
}

mod directives;
mod repo;

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
