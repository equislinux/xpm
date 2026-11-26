//! Per-command usage text.

pub(super) fn print_help_command(cmd: &str) {
    match cmd {
        "sync" | "Sy" => println!(
            r#"xpm sync — Synchronize Package Databases

USAGE:
    xpm sync [OPTIONS]
    xpm Sy [OPTIONS]

DESCRIPTION:
    Downloads the latest package database files from all configured
    repositories. This should be run before installing or upgrading
    packages to ensure you have the latest version information.

OPTIONS:
    -f, --force     Force a full database refresh even if local
                    databases appear to be up to date

EXAMPLES:
    xpm sync            # Normal sync
    xpm sync --force    # Force full refresh
    xpm Sy -f           # Same as above
"#
        ),
        "install" | "S" => println!(
            r#"xpm install — Install Packages

USAGE:
    xpm install <PACKAGES>... [OPTIONS]
    xpm S <PACKAGES>... [OPTIONS]

DESCRIPTION:
    Install one or more packages from the synchronized databases.
    Dependencies are resolved automatically, including virtual `provides`.
    Use `name=version` or `repo/name` to disambiguate a target.

ARGUMENTS:
    <PACKAGES>      Package names, name=version pins or repo/name qualifiers

OPTIONS:
    -w, --download-only     Download packages without installing
    -p, --print             Print the resolved plan without installing
    --as-deps               Mark as installed as a dependency
    --as-explicit           Mark as explicitly installed
    --needed                Skip packages already installed at that version
    --no-optional           Skip optional dependencies

EXAMPLES:
    xpm install firefox
    xpm install vim neovim tmux
    xpm install power-profiles-daemon
    xpm install extra/power-profiles-daemon
    xpm S -w linux linux-headers
    xpm install --as-deps libfoo
"#
        ),
        "remove" | "R" => println!(
            r#"xpm remove — Remove Packages

USAGE:
    xpm remove <PACKAGES>... [OPTIONS]
    xpm R <PACKAGES>... [OPTIONS]

DESCRIPTION:
    Remove installed packages from the system.

ARGUMENTS:
    <PACKAGES>      One or more package names to remove

OPTIONS:
    -s, --recursive     Also remove unneeded dependencies
    -d, --no-deps       Skip dependency checking
    -n, --nosave        Remove configuration files (purge)

EXAMPLES:
    xpm remove firefox
    xpm R -s vim           # Remove with unused deps
    xpm remove -n --recursive pkg
"#
        ),
        "upgrade" | "Su" => println!(
            r#"xpm upgrade — System Upgrade

USAGE:
    xpm upgrade [OPTIONS]
    xpm Su [OPTIONS]

DESCRIPTION:
    Upgrade all installed packages to their latest available versions.
    Run `xpm sync` first to get the latest database.

OPTIONS:
    --force             Force reinstall of up-to-date packages
    --ignore <PKG>      Skip specific packages (repeatable)

EXAMPLES:
    xpm upgrade
    xpm Su --ignore linux
    xpm upgrade --ignore pkg1 --ignore pkg2
"#
        ),
        "query" | "Q" => println!(
            r#"xpm query — Query Local Database

USAGE:
    xpm query [FILTER] [OPTIONS]
    xpm Q [FILTER] [OPTIONS]

DESCRIPTION:
    Query the local package database for installed packages. FILTER matches
    the package name exactly; use --search for a substring match.

ARGUMENTS:
    [FILTER]        Exact package name to query

OPTIONS:
    -e, --explicit      List only explicitly installed packages
    -d, --deps          List only packages installed as dependencies
    -t, --orphans       List orphan packages (no longer required)
    -u, --upgrades      List packages with available updates
    -q, --quiet         Print only package names
        --search        Treat FILTER as a substring
        --owner PATH    Show which package owns PATH
    -m, --foreign       List packages absent from all sync databases
        --unrequired    List packages no other package directly requires

EXAMPLES:
    xpm query               # List all installed
    xpm Q -e                # Explicit packages only
    xpm query --orphans     # Find orphans
    xpm query --unrequired --deps  # Unneeded dependency packages
    xpm query --owner /usr/bin/vim
    xpm query --foreign
    xpm Q -u                # List upgradeable
"#
        ),
        "check" | "Qk" => println!(
            r#"xpm check — Verify Installed Files

USAGE:
    xpm check [--full] [PACKAGES]...
    xpm Qk [--full] [PACKAGES]...

DESCRIPTION:
    Verify the files recorded in each installed package's local-database
    manifest. Fast mode checks existence and file/directory type. --full also
    verifies the stored .MTREE SHA-256 digests, symlink targets and modes.

OPTIONS:
    --full          Verify hashes, symlinks and modes (-Qkk)

EXAMPLES:
    xpm check
    xpm check --full
    xpm check hello
"#
        ),
        "mark" | "D" => println!(
            r#"xpm mark — Change Install Reason

USAGE:
    xpm mark --as-deps <PACKAGES>...
    xpm mark --as-explicit <PACKAGES>...

DESCRIPTION:
    Change the recorded install reason without reinstalling the package.

OPTIONS:
    --as-deps       Mark as installed as a dependency
    --as-explicit   Mark as explicitly installed
"#
        ),
        "deptest" | "T" => println!(
            r#"xpm deptest — Test Dependencies

USAGE:
    xpm deptest <DEPENDENCIES>...

DESCRIPTION:
    Test dependency specifications against the installed packages, including
    versioned provides. Unmet specifications are printed one per line and the
    command exits with status 127.

EXAMPLES:
    xpm deptest glibc bash
    xpm deptest 'python>=3.12'
"#
        ),
        "search" | "Ss" => println!(
            r#"xpm search — Search Packages

USAGE:
    xpm search <QUERY> [OPTIONS]
    xpm Ss <QUERY> [OPTIONS]

DESCRIPTION:
    Search for packages in the synchronized databases by name,
    description, or provides.

ARGUMENTS:
    <QUERY>         Search term

OPTIONS:
    -l, --local     Search in local database instead of sync

EXAMPLES:
    xpm search firefox
    xpm Ss "text editor"
    xpm search --local vim
"#
        ),
        "info" | "Si" | "Qi" => println!(
            r#"xpm info — Package Information

USAGE:
    xpm info <PACKAGE> [OPTIONS]
    xpm Si <PACKAGE> [OPTIONS]

DESCRIPTION:
    Display detailed information about a package including version,
    description, dependencies, and more.

ARGUMENTS:
    <PACKAGE>       Package name to inspect

OPTIONS:
    -l, --local     Query local database instead of sync

EXAMPLES:
    xpm info linux
    xpm Si firefox
    xpm info --local vim
"#
        ),
        "files" | "Ql" => println!(
            r#"xpm files — List Package Files

USAGE:
    xpm files <PACKAGE>
    xpm Ql <PACKAGE>

DESCRIPTION:
    List all files owned by an installed package.

ARGUMENTS:
    <PACKAGE>       Package name

EXAMPLES:
    xpm files bash
    xpm Ql linux
"#
        ),
        "repo" => println!(
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
        ),
        "history" => println!(
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
        ),
        "rollback" => println!(
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
        ),
        "diff" => println!(
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
        ),
        _ => println!(
            "Unknown command or topic: {cmd}\n\n\
             Use `xpm help commands` to see all available commands.\n\
             Use `xpm help` for general help."
        ),
    }
}
