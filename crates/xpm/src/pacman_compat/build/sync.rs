//! `-S` sync/install/upgrade translation.

use super::*;

pub(super) fn build_sync_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    if modifiers
        .iter()
        .any(|c| !matches!(c, 'y' | 'u' | 'p' | 's' | 'i' | 'w' | 'g' | 'l' | 'c' | 'q'))
    {
        return None;
    }

    let y = count(modifiers, 'y');
    let u = count(modifiers, 'u');
    let p = count(modifiers, 'p');
    let s = count(modifiers, 's');
    let i = count(modifiers, 'i');
    let w = count(modifiers, 'w');
    let g = count(modifiers, 'g');
    let l = count(modifiers, 'l');
    let c = count(modifiers, 'c');
    let q = count(modifiers, 'q');

    // Read-only sync listings.
    if l > 0 {
        if l > 1 || y > 0 || u > 0 || p > 0 || s > 0 || i > 0 || w > 0 || g > 0 || c > 0 || q > 1 {
            return None;
        }
        if has_install_flags(flags) || has_upgrade_flags(flags) || has_remove_flags(flags) {
            return None;
        }
        let mut command = vec![OsString::from("list")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    if g > 0 {
        if g > 2 || y > 0 || u > 0 || p > 0 || s > 0 || i > 0 || w > 0 || l > 0 || c > 0 || q > 1 {
            return None;
        }
        if has_install_flags(flags) || has_upgrade_flags(flags) || has_remove_flags(flags) {
            return None;
        }
        let mut command = vec![OsString::from("groups")];
        if g == 1 {
            command.push(OsString::from("--names"));
        }
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    if c > 0 {
        if c > 2 || y > 0 || u > 0 || p > 0 || s > 0 || i > 0 || w > 0 || l > 0 || g > 0 || q > 0 {
            return None;
        }
        if has_install_flags(flags) || has_upgrade_flags(flags) || has_remove_flags(flags) {
            return None;
        }
        let mut command = vec![OsString::from("clean")];
        if c == 2 {
            command.push(OsString::from("--all"));
        }
        return Some(command);
    }

    if s > 0 {
        if s > 1 || y > 0 || u > 0 || p > 0 || i > 0 || w > 0 || q > 0 {
            return None;
        }
        if has_install_flags(flags) || has_upgrade_flags(flags) || has_remove_flags(flags) {
            return None;
        }
        return Some(vec![OsString::from("search")]);
    }

    if i > 0 {
        if i > 1 || y > 0 || u > 0 || p > 0 || s > 0 || w > 0 || q > 0 {
            return None;
        }
        if has_install_flags(flags) || has_upgrade_flags(flags) || has_remove_flags(flags) {
            return None;
        }
        return Some(vec![OsString::from("info")]);
    }

    if u > 0 {
        // Upgrade only installs newer versions, so --needed is inherently
        // satisfied; --downloadonly is supported by `xpm upgrade`.
        if flags.as_deps || flags.as_explicit || flags.nodeps {
            return None;
        }
        let mut command = vec![OsString::from("upgrade")];
        if flags.print || p > 0 {
            command.push(OsString::from("--print"));
        }
        if w > 0 || flags.download_only {
            command.push(OsString::from("--download-only"));
        }
        for ignore in &flags.ignore {
            command.push(OsString::from("--ignore"));
            command.push(ignore.clone());
        }
        for group in &flags.ignore_groups {
            command.push(OsString::from("--ignoregroup"));
            command.push(group.clone());
        }
        for spec in &flags.assume_installed {
            command.push(OsString::from("--assume-installed"));
            command.push(spec.clone());
        }
        for pattern in &flags.overwrite {
            command.push(OsString::from("--overwrite"));
            command.push(pattern.clone());
        }
        return Some(command);
    }

    if y > 0 {
        if p > 0
            || flags.print
            || flags.nodeps
            || w > 0
            || q > 0
            || !flags.ignore.is_empty()
            || !flags.ignore_groups.is_empty()
            || !flags.assume_installed.is_empty()
        {
            return None;
        }
        if flags.as_deps
            || flags.as_explicit
            || flags.download_only
            || flags.needed
            || !flags.overwrite.is_empty()
        {
            return None;
        }
        let mut command = vec![OsString::from("sync")];
        if y > 1 {
            command.push(OsString::from("--force"));
        }
        return Some(command);
    }

    if q > 0 {
        return None;
    }

    if flags.ignore.is_empty() && !flags.nodeps {
        let mut command = build_install_command(flags)?;
        if w > 0 && !flags.download_only {
            command.insert(1, OsString::from("--download-only"));
        }
        if p > 0 && !flags.print {
            command.insert(1, OsString::from("--print"));
        }
        return Some(command);
    }

    None
}
