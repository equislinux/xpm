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

mod build;
mod flags;
mod rewrite;

#[cfg(test)]
mod tests;

use build::build_command;
use flags::*;
use rewrite::try_rewrite;

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
