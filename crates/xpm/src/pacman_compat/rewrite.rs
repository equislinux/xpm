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

fn build_files_db_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
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

fn build_mark_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
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

fn build_deptest_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    if !modifiers.is_empty()
        || has_install_flags(flags)
        || has_upgrade_flags(flags)
        || has_remove_flags(flags)
    {
        return None;
    }

    Some(vec![OsString::from("deptest")])
}

fn build_sync_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
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

fn build_install_command(flags: &Flags) -> Option<Vec<OsString>> {
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

fn build_query_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
    if modifiers.iter().any(|c| {
        !matches!(
            c,
            'e' | 'd' | 'l' | 'i' | 'q' | 'u' | 't' | 'o' | 'k' | 'g' | 'm' | 'n' | 'c' | 'p'
        )
    }) {
        return None;
    }

    if has_install_flags(flags) || has_upgrade_flags(flags) || has_remove_flags(flags) {
        return None;
    }

    let e = count(modifiers, 'e');
    let d = count(modifiers, 'd');
    let l = count(modifiers, 'l');
    let i = count(modifiers, 'i');
    let q = count(modifiers, 'q');
    let u = count(modifiers, 'u');
    let t = count(modifiers, 't');
    let o = count(modifiers, 'o');
    let k = count(modifiers, 'k');
    let g = count(modifiers, 'g');
    let m = count(modifiers, 'm');
    let n = count(modifiers, 'n');
    let c = count(modifiers, 'c');
    let p = count(modifiers, 'p');

    // -Qc is an action: the package name follows as a positional argument.
    if c > 0 {
        if c > 1 || e + d + l + i + u + t + o + k + g + m + n + p > 0 || q > 1 {
            return None;
        }
        let mut command = vec![OsString::from("query"), OsString::from("--changelog")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    // -Qp takes the package file as the next positional argument. Flags that
    // consume a value must therefore come before `--file`.
    if p > 0 {
        if p > 1 || e + d + l + i + u + t + o + k + g + m + n + c > 0 || q > 1 {
            return None;
        }
        let mut command = vec![OsString::from("query")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        command.push(OsString::from("--file"));
        return Some(command);
    }

    if o > 0 {
        if o > 1 || e + d + l + i + u + t + k + g + m + n > 0 {
            return None;
        }
        let mut command = vec![OsString::from("query")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        command.push(OsString::from("--owner"));
        return Some(command);
    }

    if k > 0 {
        if k > 2 || e + d + l + i + q + u + t + o + g + m + n > 0 {
            return None;
        }
        let mut command = vec![OsString::from("check")];
        if k == 2 {
            command.push(OsString::from("--full"));
        }
        return Some(command);
    }

    if g > 0 {
        if g > 1 || e + d + l + i + u + t + o + k + m + n > 0 {
            return None;
        }
        let mut command = vec![OsString::from("groups"), OsString::from("--local")];
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    if l > 0 {
        if l > 1 || e > 0 || d > 0 || i > 0 || u > 0 || t > 0 || q > 0 || m > 0 || n > 0 {
            return None;
        }
        return Some(vec![OsString::from("files")]);
    }

    if i > 0 {
        if i > 1 || e > 0 || d > 0 || u > 0 || t > 0 || q > 0 || m > 0 || n > 0 {
            return None;
        }
        return Some(vec![OsString::from("info")]);
    }

    if n > 1 || m > 1 {
        return None;
    }

    if u > 0 && (t > 0 || m > 0) {
        return None;
    }

    if t > 0 {
        if t > 1 {
            return None;
        }
        let mut command = vec![OsString::from("query")];
        if m > 0 {
            command.push(OsString::from("--foreign"));
        }
        if n > 0 {
            command.push(OsString::from("--native"));
        }
        command.push(OsString::from("--unrequired"));
        if d > 0 {
            command.push(OsString::from("--deps"));
        }
        if e > 0 {
            command.push(OsString::from("--explicit"));
        }
        if q > 0 {
            command.push(OsString::from("--quiet"));
        }
        return Some(command);
    }

    let mut command = vec![OsString::from("query")];
    if e > 0 {
        command.push(OsString::from("--explicit"));
    }
    if d > 0 {
        command.push(OsString::from("--deps"));
    }
    if m > 0 {
        command.push(OsString::from("--foreign"));
    }
    if n > 0 {
        command.push(OsString::from("--native"));
    }
    if u > 0 {
        command.push(OsString::from("--upgrades"));
    }
    if q > 0 {
        command.push(OsString::from("--quiet"));
    }
    Some(command)
}

fn build_remove_command(modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
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
