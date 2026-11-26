//! Pacman flag/bundle parsing.

use super::*;

#[derive(Default)]
pub(super) struct Flags {
    pub(super) globals: Vec<OsString>,
    pub(super) no_confirm: bool,
    pub(super) no_color: bool,
    pub(super) noscriptlet: bool,
    pub(super) needed: bool,
    pub(super) print: bool,
    pub(super) overwrite: Vec<OsString>,
    pub(super) as_deps: bool,
    pub(super) as_explicit: bool,
    pub(super) download_only: bool,
    pub(super) ignore: Vec<OsString>,
    pub(super) ignore_groups: Vec<OsString>,
    pub(super) assume_installed: Vec<OsString>,
    pub(super) nodeps: bool,
    pub(super) help: bool,
}

pub(super) struct Bundle {
    pub(super) operation: char,
    pub(super) modifiers: Vec<char>,
}

pub(super) fn parse_bundle(arg: &str) -> Option<Bundle> {
    let rest = arg.strip_prefix('-')?;
    let mut chars = rest.chars();
    let operation = chars.next()?;
    if !OPERATION_LETTERS.contains(&operation) {
        return None;
    }

    let modifiers: Vec<char> = chars.collect();
    if modifiers.iter().any(|c| !c.is_ascii_alphabetic()) {
        return None;
    }

    Some(Bundle {
        operation,
        modifiers,
    })
}

pub(super) fn os_starts_with_dash(value: &OsString) -> bool {
    value.to_string_lossy().starts_with('-')
}

pub(super) fn parse_long_option(
    args: &[OsString],
    index: usize,
    flags: &mut Flags,
) -> Option<usize> {
    let arg = args[index].to_str()?;
    let (name, inline_value) = arg
        .split_once('=')
        .map_or((arg, None), |(name, value)| (name, Some(value)));

    match name {
        "--root" | "--dbpath" | "--cachedir" | "--config" | "--scriptlets" | "--sysroot" => {
            let (value, next) = take_value(args, index, inline_value)?;
            if os_starts_with_dash(&value) {
                return None;
            }
            if inline_value.is_some() {
                flags.globals.push(args[index].clone());
            } else {
                flags.globals.push(args[index].clone());
                flags.globals.push(value);
            }
            Some(next)
        }
        "--ignore" => {
            let (value, next) = take_value(args, index, inline_value)?;
            flags.ignore.push(value);
            Some(next)
        }
        "--ignoregroup" => {
            let (value, next) = take_value(args, index, inline_value)?;
            flags.ignore_groups.push(value);
            Some(next)
        }
        "--assume-installed" => {
            let (value, next) = take_value(args, index, inline_value)?;
            flags.assume_installed.push(value);
            Some(next)
        }
        "--overwrite" => {
            let (value, next) = take_value(args, index, inline_value)?;
            flags.overwrite.push(value);
            Some(next)
        }
        "--color" => {
            let (value, next) = take_value(args, index, inline_value)?;
            if value == "never" {
                flags.no_color = true;
                Some(next)
            } else {
                None
            }
        }
        "--noprogressbar" => Some(index + 1),
        "--noconfirm" | "--no-confirm" => {
            require_no_inline_value(inline_value)?;
            flags.no_confirm = true;
            Some(index + 1)
        }
        "--no-color" => {
            require_no_inline_value(inline_value)?;
            flags.no_color = true;
            Some(index + 1)
        }
        "--noscriptlet" => {
            require_no_inline_value(inline_value)?;
            flags.noscriptlet = true;
            Some(index + 1)
        }
        "--needed" => {
            require_no_inline_value(inline_value)?;
            flags.needed = true;
            Some(index + 1)
        }
        "--print" => {
            require_no_inline_value(inline_value)?;
            flags.print = true;
            Some(index + 1)
        }
        "--asdeps" => {
            require_no_inline_value(inline_value)?;
            flags.as_deps = true;
            Some(index + 1)
        }
        "--asexplicit" => {
            require_no_inline_value(inline_value)?;
            flags.as_explicit = true;
            Some(index + 1)
        }
        "--downloadonly" => {
            require_no_inline_value(inline_value)?;
            flags.download_only = true;
            Some(index + 1)
        }
        "--nodeps" => {
            require_no_inline_value(inline_value)?;
            flags.nodeps = true;
            Some(index + 1)
        }
        "--verbose" | "--no-alpm-hooks" | "--shell-integration" | "--no-shell-integration" => {
            require_no_inline_value(inline_value)?;
            flags.globals.push(args[index].clone());
            Some(index + 1)
        }
        "--help" => {
            require_no_inline_value(inline_value)?;
            flags.help = true;
            Some(index + 1)
        }
        _ => None,
    }
}

pub(super) fn take_value(
    args: &[OsString],
    index: usize,
    inline_value: Option<&str>,
) -> Option<(OsString, usize)> {
    match inline_value {
        Some(value) => Some((OsString::from(value), index + 1)),
        None => Some((args.get(index + 1)?.clone(), index + 2)),
    }
}

pub(super) fn require_no_inline_value(inline_value: Option<&str>) -> Option<()> {
    if inline_value.is_some() {
        None
    } else {
        Some(())
    }
}

pub(super) fn count(modifiers: &[char], letter: char) -> usize {
    modifiers.iter().filter(|&&item| item == letter).count()
}

pub(super) fn has_install_flags(flags: &Flags) -> bool {
    flags.needed
        || flags.print
        || !flags.overwrite.is_empty()
        || flags.as_deps
        || flags.as_explicit
        || flags.download_only
        || !flags.ignore_groups.is_empty()
        || !flags.assume_installed.is_empty()
}

pub(super) fn has_upgrade_flags(flags: &Flags) -> bool {
    flags.print || !flags.overwrite.is_empty() || !flags.ignore.is_empty()
}

pub(super) fn has_remove_flags(flags: &Flags) -> bool {
    flags.nodeps
}
