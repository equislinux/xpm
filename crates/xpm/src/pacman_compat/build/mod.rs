//! Pacman-to-xpm argument rewriting.

use super::*;

pub(super) fn build_command(
    operation: char,
    modifiers: &[char],
    flags: &Flags,
) -> Option<Vec<OsString>> {
    match operation {
        'S' => build_sync_command(modifiers, flags),
        'Q' => build_query_command(modifiers, flags),
        'R' => build_remove_command(modifiers, flags),
        'U' => {
            if !modifiers.is_empty() || !flags.ignore.is_empty() || flags.nodeps {
                return None;
            }
            build_install_command(flags)
        }
        'V' => {
            if !modifiers.is_empty() {
                return None;
            }
            Some(vec![OsString::from("--version")])
        }
        'D' => build_mark_command(modifiers, flags),
        'T' => build_deptest_command(modifiers, flags),
        'F' => build_files_db_command(modifiers, flags),
        _ => None,
    }
}

mod deptest;
mod files_db;
mod install;
mod mark;
mod query;
mod remove;
mod sync;

use deptest::build_deptest_command;
use files_db::build_files_db_command;
use install::build_install_command;
use mark::build_mark_command;
use query::build_query_command;
use remove::build_remove_command;
use sync::build_sync_command;
