//! Pacman-style argv compatibility frontend.
//!
//! This module rewrites common `pacman` invocations (`-Syu`, `-Rns`, `-Ql`,
//! ...) into native `xpm` subcommands before clap parses the command line.
//! It intentionally does not replace the native CLI: anything that is already
//! valid xpm syntax is returned untouched.
//!
//! The rewriter is pure and deterministic. Unsupported pacman operations or
//! flags are returned unchanged so clap can report them instead of silently
//! dropping meaning. Implemented query/repair operations include `-Qo`,
//! `-Qm`/`-Qn`, `-Qc`, `-Qp`, `-Qq`, `-Qt`/`-Qdt`, `-Qg`, `-Qk`/`-Qkk`,
//! `-D --asdeps`, `-D --asexplicit`, `-T`, `-Sl`, `-Sg`/`-Sgg`, `-Sc`/`-Scc`,
//! `-Sw`/`-Suw`, and basic `-F`/`-Fl`/`-Fq`/`-Fy` files-database queries.
//! `-Fx` is left unchanged because xpm does not link a regular-expression
//! engine; treating an ERE as a literal path would silently change meaning.
//!
//! `--color never` maps to `--no-color`; other color modes are left for clap
//! to reject.

use std::ffi::{OsStr, OsString};

/// Native xpm subcommands and the aliases clap already understands.
const NATIVE_TOKENS: &[&str] = &[
    "sync",
    "install",
    "remove",
    "upgrade",
    "query",
    "search",
    "info",
    "files",
    "repo",
    "history",
    "rollback",
    "diff",
    "import-pacman",
    "sync-pacman-db",
    "check",
    "mark",
    "deptest",
    "groups",
    "list",
    "clean",
    "file-db",
    "usage",
    "Sy",
    "S",
    "R",
    "Su",
    "Q",
    "Ss",
    "Si",
    "Ql",
];

/// Long options that consume the following argv element. This is used only for
/// locating the first non-global token; it intentionally includes a few pacman
/// options that xpm does not implement so their value is not mistaken for a
/// subcommand name.
const LONG_VALUE_OPTIONS: &[&str] = &[
    "--root",
    "--dbpath",
    "--cachedir",
    "--config",
    "--scriptlets",
    "--ignore",
    "--overwrite",
    "--color",
    "--gpgdir",
    "--hookdir",
    "--logfile",
    "--ignoregroup",
    "--assume-installed",
    "--arch",
    "--sysroot",
];

/// Short global options that consume a value.
const SHORT_VALUE_OPTIONS: &[char] = &['c', 'r', 'b'];

const OPERATION_LETTERS: &[char] = &['S', 'Q', 'R', 'U', 'D', 'T', 'F', 'V'];

/// All pacman one-letter operation modifiers. Individual operations validate
/// the subset they understand.
const MODIFIER_LETTERS: &[char] = &[
    'y', 'u', 'n', 's', 'd', 'c', 'p', 'q', 'e', 'i', 'l', 't', 'o', 'k', 'g', 'm', 'w',
];

const REMOVE_MODIFIERS: &[char] = &['n', 's', 'd', 'c', 'u'];

/// Rewrite pacman-style arguments into native xpm arguments.
///
/// `argv[0]` is preserved. Native xpm invocations are returned unchanged.
/// Unsupported pacman syntax is also returned unchanged so clap can report it.
pub fn rewrite_args(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let args: Vec<OsString> = args.into_iter().collect();
    if args.is_empty() {
        return args;
    }

    match first_non_global(&args[1..]) {
        Some(token) if !is_native_token(token) => try_rewrite(&args).unwrap_or(args),
        _ => args,
    }
}

fn is_native_token(token: &OsStr) -> bool {
    token
        .to_str()
        .is_some_and(|token| NATIVE_TOKENS.contains(&token))
}

/// Find the first argument that is neither a global option nor its value.
fn first_non_global(args: &[OsString]) -> Option<&OsStr> {
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        let Some(arg_str) = arg.to_str() else {
            return Some(arg);
        };

        if arg_str == "--" {
            return args.get(index + 1).map(OsString::as_os_str);
        }

        if arg_str.starts_with("--") {
            let name = arg_str.split_once('=').map_or(arg_str, |(name, _)| name);
            let has_inline_value = arg_str.contains('=');
            if !has_inline_value && LONG_VALUE_OPTIONS.contains(&name) {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if let Some(rest) = arg_str.strip_prefix('-') {
            let Some(first) = rest.chars().next() else {
                return Some(arg);
            };
            if OPERATION_LETTERS.contains(&first) {
                return Some(arg);
            }
            if SHORT_VALUE_OPTIONS.contains(&first) {
                if rest.chars().count() == 1 {
                    index += 2;
                } else {
                    index += 1;
                }
                continue;
            }
            index += 1;
            continue;
        }

        return Some(arg);
    }

    None
}

#[derive(Default)]
struct Flags {
    globals: Vec<OsString>,
    no_confirm: bool,
    no_color: bool,
    noscriptlet: bool,
    needed: bool,
    print: bool,
    overwrite: Vec<OsString>,
    as_deps: bool,
    as_explicit: bool,
    download_only: bool,
    ignore: Vec<OsString>,
    ignore_groups: Vec<OsString>,
    assume_installed: Vec<OsString>,
    nodeps: bool,
    help: bool,
}

struct Bundle {
    operation: char,
    modifiers: Vec<char>,
}

fn parse_bundle(arg: &str) -> Option<Bundle> {
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

fn try_rewrite(args: &[OsString]) -> Option<Vec<OsString>> {
    let mut flags = Flags::default();
    let mut operation = None;
    let mut modifiers = Vec::new();
    let mut pending_modifiers = Vec::new();
    let mut positionals = Vec::new();
    let mut after_double_dash = false;
    let mut saw_double_dash = false;
    let mut index = 1;

    while index < args.len() {
        let arg = &args[index];

        if after_double_dash {
            positionals.push(arg.clone());
            index += 1;
            continue;
        }

        let Some(arg_str) = arg.to_str() else {
            positionals.push(arg.clone());
            index += 1;
            continue;
        };

        if arg_str == "--" {
            after_double_dash = true;
            saw_double_dash = true;
            index += 1;
            continue;
        }

        if arg_str.starts_with("--") {
            index = parse_long_option(args, index, &mut flags)?;
            continue;
        }

        if let Some(rest) = arg_str.strip_prefix('-') {
            if rest.is_empty() {
                positionals.push(arg.clone());
                index += 1;
                continue;
            }

            if let Some(bundle) = parse_bundle(arg_str) {
                if operation.is_some() {
                    return None;
                }
                operation = Some(bundle.operation);
                modifiers = bundle.modifiers;
                index += 1;
                continue;
            }

            let chars: Vec<char> = rest.chars().collect();

            if chars.iter().all(|c| *c == 'v') {
                flags.globals.push(arg.clone());
                index += 1;
                continue;
            }

            if arg_str == "-h" {
                flags.help = true;
                index += 1;
                continue;
            }

            if chars.first() == Some(&'c') {
                if operation == Some('R') {
                    if chars.iter().all(|c| REMOVE_MODIFIERS.contains(c)) {
                        modifiers.extend(chars);
                        index += 1;
                        continue;
                    }
                    return None;
                }

                if chars.len() == 1 {
                    let value = args.get(index + 1)?.clone();
                    if os_starts_with_dash(&value) {
                        return None;
                    }
                    flags.globals.push(arg.clone());
                    flags.globals.push(value);
                    index += 2;
                    continue;
                }

                flags.globals.push(arg.clone());
                index += 1;
                continue;
            }

            if chars.len() == 1 {
                let letter = chars[0];
                if letter == 'r' || letter == 'b' {
                    let value = args.get(index + 1)?.clone();
                    if os_starts_with_dash(&value) {
                        return None;
                    }
                    flags.globals.push(arg.clone());
                    flags.globals.push(value);
                    index += 2;
                    continue;
                }
            }

            if chars.len() > 1 && (chars[0] == 'r' || chars[0] == 'b') {
                flags.globals.push(arg.clone());
                index += 1;
                continue;
            }

            if chars.iter().all(|c| MODIFIER_LETTERS.contains(c)) {
                if operation.is_some() {
                    modifiers.extend(chars);
                } else {
                    pending_modifiers.extend(chars);
                }
                index += 1;
                continue;
            }

            return None;
        }

        positionals.push(arg.clone());
        index += 1;
    }

    let operation = operation?;
    modifiers.extend(pending_modifiers);

    let command = build_command(operation, &modifiers, &flags)?;

    let mut output = Vec::with_capacity(args.len() + command.len() + 2);
    output.push(args[0].clone());
    output.extend(flags.globals);
    if flags.no_confirm {
        output.push(OsString::from("--no-confirm"));
    }
    if flags.no_color {
        output.push(OsString::from("--no-color"));
    }
    if flags.noscriptlet {
        output.push(OsString::from("--noscriptlet"));
    }
    output.extend(command);
    if flags.help {
        output.push(OsString::from("--help"));
    }
    if saw_double_dash {
        output.push(OsString::from("--"));
    }
    output.extend(positionals);

    Some(output)
}

fn os_starts_with_dash(value: &OsString) -> bool {
    value.to_string_lossy().starts_with('-')
}

fn parse_long_option(args: &[OsString], index: usize, flags: &mut Flags) -> Option<usize> {
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
        "--verbose" | "--no-pacman-mirror" | "--shell-integration" | "--no-shell-integration" => {
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

fn take_value(
    args: &[OsString],
    index: usize,
    inline_value: Option<&str>,
) -> Option<(OsString, usize)> {
    match inline_value {
        Some(value) => Some((OsString::from(value), index + 1)),
        None => Some((args.get(index + 1)?.clone(), index + 2)),
    }
}

fn require_no_inline_value(inline_value: Option<&str>) -> Option<()> {
    if inline_value.is_some() {
        None
    } else {
        Some(())
    }
}

fn count(modifiers: &[char], letter: char) -> usize {
    modifiers.iter().filter(|&&item| item == letter).count()
}

fn has_install_flags(flags: &Flags) -> bool {
    flags.needed
        || flags.print
        || !flags.overwrite.is_empty()
        || flags.as_deps
        || flags.as_explicit
        || flags.download_only
        || !flags.ignore_groups.is_empty()
        || !flags.assume_installed.is_empty()
}

fn has_upgrade_flags(flags: &Flags) -> bool {
    flags.print || !flags.overwrite.is_empty() || !flags.ignore.is_empty()
}

fn has_remove_flags(flags: &Flags) -> bool {
    flags.nodeps
}

fn build_command(operation: char, modifiers: &[char], flags: &Flags) -> Option<Vec<OsString>> {
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
        if o > 1 || e + d + l + i + q + u + t + k + g + m + n > 0 {
            return None;
        }
        return Some(vec![OsString::from("query"), OsString::from("--owner")]);
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

#[cfg(test)]
mod tests {
    use super::rewrite_args;
    use std::ffi::OsString;

    fn rewrite(input: &[&str]) -> Vec<String> {
        rewrite_args(input.iter().map(OsString::from))
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn assert_rewrite(input: &[&str], expected: &[&str]) {
        assert_eq!(rewrite(input), expected, "input: {input:?}");
    }

    #[test]
    fn rewrites_full_upgrade() {
        assert_rewrite(
            &["xpm", "-Syu", "--noconfirm"],
            &["xpm", "--no-confirm", "upgrade"],
        );
        assert_rewrite(&["xpm", "-Suy"], &["xpm", "upgrade"]);
        assert_rewrite(&["xpm", "-Su"], &["xpm", "upgrade"]);
    }

    #[test]
    fn rewrites_sync() {
        assert_rewrite(&["xpm", "-Sy"], &["xpm", "sync"]);
        assert_rewrite(&["xpm", "-Syy"], &["xpm", "sync", "--force"]);
    }

    #[test]
    fn rewrites_upgrade_ignore_after_subcommand() {
        assert_rewrite(
            &["xpm", "-Syu", "--ignore", "linux"],
            &["xpm", "upgrade", "--ignore", "linux"],
        );
    }

    #[test]
    fn rewrites_install_flags() {
        assert_rewrite(
            &["xpm", "-S", "--needed", "--noconfirm", "firefox"],
            &["xpm", "--no-confirm", "install", "--needed", "firefox"],
        );
        assert_rewrite(&["xpm", "-S", "firefox"], &["xpm", "install", "firefox"]);
    }

    #[test]
    fn rewrites_install_print() {
        assert_rewrite(
            &["xpm", "-Sp", "firefox"],
            &["xpm", "install", "--print", "firefox"],
        );
    }

    #[test]
    fn rewrites_local_package_install() {
        assert_rewrite(
            &["xpm", "-U", "/tmp/foo.pkg.tar.zst"],
            &["xpm", "install", "/tmp/foo.pkg.tar.zst"],
        );
    }

    #[test]
    fn rewrites_remove_bundles() {
        assert_rewrite(
            &["xpm", "-Rns", "pkg"],
            &["xpm", "remove", "--recursive", "--nosave", "pkg"],
        );
        assert_rewrite(
            &["xpm", "-Rsn", "pkg"],
            &["xpm", "remove", "--recursive", "--nosave", "pkg"],
        );
        assert_rewrite(&["xpm", "-Rdd", "pkg"], &["xpm", "remove", "-dd", "pkg"]);
        assert_rewrite(
            &["xpm", "-Rc", "pkg"],
            &["xpm", "remove", "--cascade", "pkg"],
        );
        assert_rewrite(
            &["xpm", "-Ru", "pkg"],
            &["xpm", "remove", "--unneeded", "pkg"],
        );
    }

    #[test]
    fn rewrites_query_variants() {
        assert_rewrite(&["xpm", "-Qe"], &["xpm", "query", "--explicit"]);
        assert_rewrite(&["xpm", "-Qd"], &["xpm", "query", "--deps"]);
        assert_rewrite(&["xpm", "-Qq"], &["xpm", "query", "--quiet"]);
        assert_rewrite(&["xpm", "-Qi", "pkg"], &["xpm", "info", "pkg"]);
        assert_rewrite(&["xpm", "-Ql", "pkg"], &["xpm", "files", "pkg"]);
    }

    #[test]
    fn rewrites_query_repair_and_database_operations() {
        assert_rewrite(
            &["xpm", "-Qo", "/usr/bin/foo"],
            &["xpm", "query", "--owner", "/usr/bin/foo"],
        );
        assert_rewrite(&["xpm", "-Qm"], &["xpm", "query", "--foreign"]);
        assert_rewrite(&["xpm", "-Qmq"], &["xpm", "query", "--foreign", "--quiet"]);
        assert_rewrite(&["xpm", "-Qt"], &["xpm", "query", "--unrequired"]);
        assert_rewrite(
            &["xpm", "-Qdt"],
            &["xpm", "query", "--unrequired", "--deps"],
        );
        assert_rewrite(
            &["xpm", "-Qtd"],
            &["xpm", "query", "--unrequired", "--deps"],
        );
        assert_rewrite(
            &["xpm", "-Qteq"],
            &["xpm", "query", "--unrequired", "--explicit", "--quiet"],
        );
        assert_rewrite(&["xpm", "-Qg"], &["xpm", "groups", "--local"]);
        assert_rewrite(&["xpm", "-Qqg"], &["xpm", "groups", "--local", "--quiet"]);
        assert_rewrite(&["xpm", "-Qk"], &["xpm", "check"]);
        assert_rewrite(&["xpm", "-Qkk"], &["xpm", "check", "--full"]);
        assert_rewrite(
            &["xpm", "-Qkk", "hello"],
            &["xpm", "check", "--full", "hello"],
        );
        assert_rewrite(
            &["xpm", "-D", "--asdeps", "hello"],
            &["xpm", "mark", "--as-deps", "hello"],
        );
        assert_rewrite(
            &["xpm", "-D", "--asexplicit", "hello"],
            &["xpm", "mark", "--as-explicit", "hello"],
        );
        assert_rewrite(
            &["xpm", "-T", "glibc>=2.39", "bash"],
            &["xpm", "deptest", "glibc>=2.39", "bash"],
        );
    }

    #[test]
    fn rewrites_sync_listing_group_and_clean_operations() {
        assert_rewrite(&["xpm", "-Sl"], &["xpm", "list"]);
        assert_rewrite(&["xpm", "-Slq"], &["xpm", "list", "--quiet"]);
        assert_rewrite(&["xpm", "-Sl", "core"], &["xpm", "list", "core"]);
        assert_rewrite(&["xpm", "-Sg"], &["xpm", "groups", "--names"]);
        assert_rewrite(&["xpm", "-Sgg"], &["xpm", "groups"]);
        assert_rewrite(
            &["xpm", "-Sgq", "desktop"],
            &["xpm", "groups", "--names", "--quiet", "desktop"],
        );
        assert_rewrite(&["xpm", "-Sc"], &["xpm", "clean"]);
        assert_rewrite(&["xpm", "-Scc"], &["xpm", "clean", "--all"]);
    }

    #[test]
    fn rewrites_download_only_sync_and_upgrade() {
        assert_rewrite(
            &["xpm", "-Sw", "firefox"],
            &["xpm", "install", "--download-only", "firefox"],
        );
        assert_rewrite(&["xpm", "-Suw"], &["xpm", "upgrade", "--download-only"]);
        assert_rewrite(
            &["xpm", "-Syu", "--downloadonly"],
            &["xpm", "upgrade", "--download-only"],
        );
    }

    #[test]
    fn rewrites_native_file_and_changelog_queries() {
        assert_rewrite(&["xpm", "-Qn"], &["xpm", "query", "--native"]);
        assert_rewrite(&["xpm", "-Qnq"], &["xpm", "query", "--native", "--quiet"]);
        assert_rewrite(
            &["xpm", "-Qnt"],
            &["xpm", "query", "--native", "--unrequired"],
        );
        assert_rewrite(
            &["xpm", "-Qc", "hello"],
            &["xpm", "query", "--changelog", "hello"],
        );
        assert_rewrite(
            &["xpm", "-Qcq", "hello"],
            &["xpm", "query", "--changelog", "--quiet", "hello"],
        );
        assert_rewrite(
            &["xpm", "-Qp", "/tmp/hello.pkg.tar.zst"],
            &["xpm", "query", "--file", "/tmp/hello.pkg.tar.zst"],
        );
        assert_rewrite(
            &["xpm", "-Qpq", "/tmp/hello.pkg.tar.zst"],
            &[
                "xpm",
                "query",
                "--quiet",
                "--file",
                "/tmp/hello.pkg.tar.zst",
            ],
        );
    }

    #[test]
    fn rewrites_files_database_queries() {
        assert_rewrite(
            &["xpm", "-F", "usr/bin/hello"],
            &["xpm", "file-db", "usr/bin/hello"],
        );
        assert_rewrite(
            &["xpm", "-Fl", "hello"],
            &["xpm", "file-db", "--list", "hello"],
        );
        assert_rewrite(
            &["xpm", "-Fq", "usr/bin/hello"],
            &["xpm", "file-db", "--quiet", "usr/bin/hello"],
        );
        assert_rewrite(&["xpm", "-Fy"], &["xpm", "file-db", "--refresh"]);
        assert_rewrite(
            &["xpm", "-Fyy"],
            &["xpm", "file-db", "--refresh", "--refresh"],
        );
    }

    #[test]
    fn rewrites_transaction_options_and_sysroot() {
        assert_rewrite(
            &[
                "xpm",
                "-S",
                "--ignoregroup",
                "kernel",
                "--assume-installed",
                "linux=6.9",
                "pkg",
            ],
            &[
                "xpm",
                "install",
                "--ignoregroup",
                "kernel",
                "--assume-installed",
                "linux=6.9",
                "pkg",
            ],
        );
        assert_rewrite(
            &["xpm", "-Syu", "--ignoregroup", "kernel"],
            &["xpm", "upgrade", "--ignoregroup", "kernel"],
        );
        assert_rewrite(
            &["xpm", "--sysroot", "/mnt", "-Syu"],
            &["xpm", "--sysroot", "/mnt", "upgrade"],
        );
    }

    #[test]
    fn rewrites_version() {
        assert_rewrite(&["xpm", "-V"], &["xpm", "--version"]);
    }

    #[test]
    fn leaves_native_invocations_unchanged() {
        assert_rewrite(&["xpm", "install", "foo"], &["xpm", "install", "foo"]);
        assert_rewrite(&["xpm", "Q", "foo"], &["xpm", "Q", "foo"]);
        assert_rewrite(
            &["xpm", "--root", "/mnt", "install", "foo"],
            &["xpm", "--root", "/mnt", "install", "foo"],
        );
    }

    #[test]
    fn leaves_unsupported_operations_unchanged() {
        for args in [
            vec!["xpm", "-Qkkk"],
            vec!["xpm", "-Qo", "-Qm"],
            vec!["xpm", "-Qgl"],
            vec!["xpm", "-D", "pkg"],
            vec!["xpm", "-D", "--asdeps", "--asexplicit", "pkg"],
            vec!["xpm", "-T", "--needed", "pkg"],
            vec!["xpm", "-Fx", "usr/bin/.*"],
            vec!["xpm", "-Fz"],
            vec!["xpm", "--gpgdir", "/tmp/gnupg", "-Syu"],
        ] {
            assert_eq!(rewrite(&args), args, "input: {args:?}");
        }
    }

    #[test]
    fn global_root_and_dbpath_pass_through() {
        assert_rewrite(
            &["xpm", "--root", "/mnt", "--dbpath", "/mnt/db", "-Syu"],
            &["xpm", "--root", "/mnt", "--dbpath", "/mnt/db", "upgrade"],
        );
        assert_rewrite(
            &["xpm", "-r", "/mnt", "-b", "/mnt/db", "-Q"],
            &["xpm", "-r", "/mnt", "-b", "/mnt/db", "query"],
        );
    }

    #[test]
    fn rewrites_global_aliases() {
        assert_rewrite(
            &["xpm", "-S", "--asdeps", "--downloadonly", "foo"],
            &["xpm", "install", "--as-deps", "--download-only", "foo"],
        );
        assert_rewrite(
            &["xpm", "-Syu", "--color", "never", "--noconfirm"],
            &["xpm", "--no-confirm", "--no-color", "upgrade"],
        );
    }

    #[test]
    fn unknown_color_mode_is_rejected() {
        let args = vec!["xpm", "-Syu", "--color", "always"];
        assert_eq!(rewrite(&args), args);
    }

    #[test]
    fn preserves_double_dash_and_positional_order() {
        assert_rewrite(
            &["xpm", "-S", "--", "-odd-name", "normal"],
            &["xpm", "install", "--", "-odd-name", "normal"],
        );
    }

    #[test]
    fn rejects_multiple_operation_bundles() {
        let args = vec!["xpm", "-Syu", "-Rns", "pkg"];
        assert_eq!(rewrite(&args), args);
    }

    #[test]
    fn rewritten_commands_parse_with_clap() {
        use crate::cli::Cli;
        use clap::Parser;

        let cases = [
            vec!["xpm", "-Syu", "--noconfirm"],
            vec!["xpm", "-Sy"],
            vec!["xpm", "-Syy"],
            vec!["xpm", "-Syu", "--ignore", "linux"],
            vec!["xpm", "-S", "--needed", "--noconfirm", "firefox"],
            vec!["xpm", "-Sp", "firefox"],
            vec!["xpm", "-U", "/tmp/foo.pkg.tar.zst"],
            vec!["xpm", "-Rns", "pkg"],
            vec!["xpm", "-Rdd", "pkg"],
            vec!["xpm", "-R", "-ns", "pkg"],
            vec!["xpm", "-Rc", "pkg"],
            vec!["xpm", "-Ru", "pkg"],
            vec!["xpm", "-Qe"],
            vec!["xpm", "-Qd"],
            vec!["xpm", "-Qq"],
            vec!["xpm", "-Qo", "/usr/bin/foo"],
            vec!["xpm", "-Qm"],
            vec!["xpm", "-Qmq"],
            vec!["xpm", "-Qt"],
            vec!["xpm", "-Qdt"],
            vec!["xpm", "-Qg"],
            vec!["xpm", "-Qqg"],
            vec!["xpm", "-Qk"],
            vec!["xpm", "-Qkk"],
            vec!["xpm", "-Qkk", "hello"],
            vec!["xpm", "-D", "--asdeps", "hello"],
            vec!["xpm", "-D", "--asexplicit", "hello"],
            vec!["xpm", "-T", "glibc>=2.39"],
            vec!["xpm", "-T"],
            vec!["xpm", "-Qi", "pkg"],
            vec!["xpm", "-Ql", "pkg"],
            vec!["xpm", "-Ss", "query"],
            vec!["xpm", "-Si", "pkg"],
            vec!["xpm", "-S", "--asdeps", "--downloadonly", "foo"],
            vec!["xpm", "-Syu", "--color", "never"],
            vec!["xpm", "--root", "/mnt", "--dbpath", "/mnt/db", "-Syu"],
            vec!["xpm", "-S", "--", "-odd-name"],
            vec!["xpm", "-Sw", "firefox"],
            vec!["xpm", "-Suw"],
            vec!["xpm", "-Sl"],
            vec!["xpm", "-Slq", "core"],
            vec!["xpm", "-Sg"],
            vec!["xpm", "-Sgg"],
            vec!["xpm", "-Sgq", "desktop"],
            vec!["xpm", "-Sc"],
            vec!["xpm", "-Scc"],
            vec!["xpm", "-Qn"],
            vec!["xpm", "-Qnq"],
            vec!["xpm", "-Qnt"],
            vec!["xpm", "-Qc", "hello"],
            vec!["xpm", "-Qcq", "hello"],
            vec!["xpm", "-Qp", "/tmp/hello.pkg.tar.zst"],
            vec!["xpm", "-Qpq", "/tmp/hello.pkg.tar.zst"],
            vec!["xpm", "-F", "usr/bin/hello"],
            vec!["xpm", "-Fl", "hello"],
            vec!["xpm", "-Fq", "usr/bin/hello"],
            vec!["xpm", "-Fy"],
            vec!["xpm", "-Fyy"],
            vec!["xpm", "-S", "--ignoregroup", "kernel", "pkg"],
            vec!["xpm", "-Syu", "--ignoregroup", "kernel"],
            vec!["xpm", "-S", "--assume-installed", "linux=6.9", "pkg"],
            vec!["xpm", "--sysroot", "/mnt", "-Syu"],
        ];

        for input in cases {
            let rewritten = rewrite_args(input.iter().map(OsString::from));
            let display = rewritten
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            if let Err(err) = Cli::try_parse_from(rewritten) {
                panic!("{input:?} rewrote to `{display}`, which clap rejected: {err}");
            }
        }
    }
}
