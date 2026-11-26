//! Help text for sync/install/remove/upgrade.

pub(super) fn print_sync() {
    println!(
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
    );
}

pub(super) fn print_install() {
    println!(
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
    );
}

pub(super) fn print_remove() {
    println!(
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
    );
}

pub(super) fn print_upgrade() {
    println!(
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
    );
}
