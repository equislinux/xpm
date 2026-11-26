//! Help text for repo/history/rollback/diff.

pub(super) fn print_repo() {
    println!(
        r#"xpm repo — Repository Management

USAGE:
    xpm repo <ACTION>

ACTIONS:
    list                    List all active repositories
    add <name> <url>        Add a user repository
    remove <name>           Remove a user repository

EXAMPLES:
    xpm repo list
    xpm repo add chaotic-aur https://cdn-mirror.chaotic.cx/$repo/$arch
    xpm repo remove chaotic-aur

See `xpm help repos` for more details on repository configuration.
"#
    );
}

pub(super) fn print_history() {
    println!(
        r#"xpm history — Transaction Journal

USAGE:
    xpm history [--json]

DESCRIPTION:
    Show past xpm transactions, newest first. When generations are
    available, each entry shows the generation it produced (`gen:NNNN`).

OPTIONS:
    --json          One JSON object per transaction (machine output)

EXAMPLES:
    xpm history
    xpm history --json
"#
    );
}

pub(super) fn print_rollback() {
    println!(
        r#"xpm rollback — Undo the Last Transaction

USAGE:
    xpm rollback [--last | --journal <ID>] [--dry-run]

DESCRIPTION:
    Compute the inverse of a successful transaction and replay it using
    the local package cache. This is the package-level recovery path;
    whole-system recovery remains `x gen rollback`.

OPTIONS:
    --last              Undo the newest successful transaction (default)
    --journal <ID>      Undo a specific journal id (see `xpm history`)
    --dry-run           Print the inverse plan without changing anything

EXAMPLES:
    xpm rollback --dry-run
    xpm rollback --last
    xpm rollback --journal 1727900000-1234
"#
    );
}

pub(super) fn print_diff() {
    println!(
        r#"xpm diff — Compare Against a Generation

USAGE:
    xpm diff <GENERATION> [--json]

DESCRIPTION:
    Compare the installed packages (local database) against the
    `packages.tsv` capture of a generation. Use `current` for the
    default generation.

OPTIONS:
    --json              Emit a single JSON object

EXAMPLES:
    xpm diff 0003
    xpm diff current --json
"#
    );
}
