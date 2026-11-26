//! `-R` removal translation.

use super::*;

pub(super) fn build_remove_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    if modifiers.iter().any(|c| !REMOVE_MODIFIERS.contains(c)) {
        return None;
    }

    if has_install_flags(flags) || has_upgrade_flags(flags) {
        return None;
    }

    let recursive = count(modifiers, 's');
    let no_deps = count(modifiers, 'd');

    let mut command = vec![OsString::from("remove")];
    for _ in 0..recursive {
        command.push(OsString::from("--recursive"));
    }
    if count(modifiers, 'c') > 0 {
        command.push(OsString::from("--cascade"));
    }
    if count(modifiers, 'u') > 0 {
        command.push(OsString::from("--unneeded"));
    }
    if no_deps > 0 {
        command.push(OsString::from(format!("-{}", "d".repeat(no_deps))));
    }
    if flags.nodeps {
        command.push(OsString::from("--nodeps"));
    }
    if count(modifiers, 'n') > 0 {
        command.push(OsString::from("--nosave"));
    }
    Some(command)
}
