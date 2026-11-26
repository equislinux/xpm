//! Help text for query/check/mark/deptest.

pub(super) fn print_query() {
    println!(
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
    );
}

pub(super) fn print_check() {
    println!(
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
    );
}

pub(super) fn print_mark() {
    println!(
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
    );
}

pub(super) fn print_deptest() {
    println!(
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
    );
}
