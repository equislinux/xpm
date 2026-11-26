//! `-U` local-package install translation.

use super::*;

pub(super) fn build_install_command(flags: &Flags) -> Option<Vec<OsString>> {
    if !flags.ignore.is_empty() || flags.nodeps {
        return None;
    }

    let mut command = vec![OsString::from("install")];
    if flags.needed {
        command.push(OsString::from("--needed"));
    }
    if flags.as_deps {
        command.push(OsString::from("--as-deps"));
    }
    if flags.as_explicit {
        command.push(OsString::from("--as-explicit"));
    }
    if flags.download_only {
        command.push(OsString::from("--download-only"));
    }
    if flags.print {
        command.push(OsString::from("--print"));
    }
    for pattern in &flags.overwrite {
        command.push(OsString::from("--overwrite"));
        command.push(pattern.clone());
    }
    for group in &flags.ignore_groups {
        command.push(OsString::from("--ignoregroup"));
        command.push(group.clone());
    }
    for spec in &flags.assume_installed {
        command.push(OsString::from("--assume-installed"));
        command.push(spec.clone());
    }
    Some(command)
}
