//! `Include` directive expansion.

use super::*;

pub(super) fn expand_include(pattern: &str, file: &Path, line: usize) -> XpmResult<Vec<PathBuf>> {
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
