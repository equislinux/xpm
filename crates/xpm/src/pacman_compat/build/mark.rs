//! `-D` install-reason translation.

use super::*;

pub(super) fn build_mark_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    if !modifiers.is_empty()
        || flags.needed
        || flags.print
        || !flags.overwrite.is_empty()
        || flags.download_only
        || !flags.ignore.is_empty()
        || !flags.ignore_groups.is_empty()
        || !flags.assume_installed.is_empty()
        || flags.nodeps
    {
        return None;
    }

    match (flags.as_deps, flags.as_explicit) {
        (true, false) => Some(vec![OsString::from("mark"), OsString::from("--as-deps")]),
        (false, true) => Some(vec![
            OsString::from("mark"),
            OsString::from("--as-explicit"),
        ]),
        _ => None,
    }
}
