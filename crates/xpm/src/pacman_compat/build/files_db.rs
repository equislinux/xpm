//! `-F` files-database translation.

use super::*;

pub(super) fn build_files_db_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    // `-Fx` would need a regular-expression engine that xpm does not link.
    // Leave it unchanged so clap reports the unsupported option rather than
    // silently treating an ERE as a literal path.
    if modifiers
        .iter()
        .any(|c| !matches!(c, 'l' | 'y' | 'q' | 'v'))
    {
        return None;
    }

    if !flags.ignore.is_empty()
        || has_install_flags(flags)
        || has_upgrade_flags(flags)
        || has_remove_flags(flags)
    {
        return None;
    }

    let list = count(modifiers, 'l');
    let refresh = count(modifiers, 'y');
    let quiet = count(modifiers, 'q');
    let verbose = count(modifiers, 'v');

    if list > 1 || quiet > 1 || verbose > 1 {
        return None;
    }

    let mut command = vec![OsString::from("file-db")];
    for _ in 0..refresh {
        command.push(OsString::from("--refresh"));
    }
    if list > 0 {
        command.push(OsString::from("--list"));
    }
    if quiet > 0 {
        command.push(OsString::from("--quiet"));
    }
    if verbose > 0 {
        command.push(OsString::from("--verbose"));
    }
    Some(command)
}
