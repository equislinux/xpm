//! Per-command usage text.

mod inspect;
mod query;
mod repo;
mod sync;

pub(super) fn print_help_command(cmd: &str) {
    match cmd {
        "sync" | "Sy" => sync::print_sync(),
        "install" | "S" => sync::print_install(),
        "remove" | "R" => sync::print_remove(),
        "upgrade" | "Su" => sync::print_upgrade(),
        "query" | "Q" => query::print_query(),
        "check" | "Qk" => query::print_check(),
        "mark" | "D" => query::print_mark(),
        "deptest" | "T" => query::print_deptest(),
        "search" | "Ss" => inspect::print_search(),
        "info" | "Si" | "Qi" => inspect::print_info(),
        "files" | "Ql" => inspect::print_files(),
        "repo" => repo::print_repo(),
        "history" => repo::print_history(),
        "rollback" => repo::print_rollback(),
        "diff" => repo::print_diff(),
        _ => println!(
            "Unknown command or topic: {cmd}\n\n\
             Use `xpm help commands` to see all available commands.\n\
             Use `xpm help` for general help."
        ),
    }
}
