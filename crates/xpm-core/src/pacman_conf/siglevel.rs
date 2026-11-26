//! SigLevel and option value parsers.

use super::*;

pub(super) fn parse_sig_level(
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

pub(super) fn parse_usage(
    value: &str,
    initial: Usage,
    file: &Path,
    line: usize,
) -> XpmResult<Option<Usage>> {
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

pub(super) fn parse_parallel_downloads(value: &str, file: &Path, line: usize) -> XpmResult<u32> {
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
