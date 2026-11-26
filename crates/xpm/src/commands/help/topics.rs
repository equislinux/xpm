//! Usage help topics: overview, command list, configuration and repositories.

pub(super) fn print_help_overview() {
    println!(
        r#"xpm — Modern package manager for X Distribution

USAGE:
    xpm <COMMAND> [OPTIONS]
    xpm <ALIAS> [OPTIONS]

QUICK START:
    xpm sync                Synchronize package databases
    xpm install <pkg>       Install a package
    xpm remove <pkg>        Remove a package
    xpm upgrade             Upgrade all packages
    xpm search <query>      Search for packages

TOPICS:
    xpm usage commands      List all available commands
    xpm usage config        Configuration file format
    xpm usage repos         Repository management
    xpm usage <command>     Help for a specific command

GLOBAL FLAGS:
    -c, --config <PATH>     Custom configuration file
    -v, --verbose           Increase verbosity (-v, -vv, -vvv)
    --no-confirm            Skip confirmation prompts
    --root <PATH>           Alternative installation root
    --dbpath <PATH>         Alternative database directory
    --cachedir <PATH>       Alternative cache directory
    --no-color              Disable colored output
    --no-alpm-hooks         Skip pacman-style .hook files

PACMAN ALIASES:
    Sy → sync     S → install    R → remove     Su → upgrade
    Q  → query    Ss → search    Si → info      Ql → files
    Qk → check    D → mark       T → deptest

DOCUMENTATION:
    Full CLI reference: docs/CLI.md
    Configuration:      /etc/xpm.conf
    User repos:         /etc/xpm.d/
"#
    );
}

pub(super) fn print_help_commands() {
    println!(
        r#"xpm — Available Commands

PACKAGE OPERATIONS:
    sync        Synchronize package databases from mirrors
    install     Install one or more packages
    remove      Remove installed packages
    upgrade     Upgrade all installed packages

QUERIES:
    query       Query the local package database
    search      Search for packages in sync databases
    info        Display detailed package information
    files       List files owned by a package
    check       Verify installed files against the local database
    deptest     Test dependency specifications against installed packages
    history     Show the transaction journal
    rollback    Undo the last transaction (package level, from the cache)
    diff        Compare installed packages against a generation capture

DATABASE MAINTENANCE:
    mark        Change install reasons without reinstalling

REPOSITORY MANAGEMENT:
    repo add    Add a temporary repository
    repo remove Remove a user-added repository
    repo list   List all active repositories

HELP:
    usage       Display detailed usage information

For detailed help on any command:
    xpm usage <command>
    xpm <command> --help
"#
    );
}

pub(super) fn print_help_config() {
    println!(
        r#"xpm — Configuration

CONFIGURATION FILE:
    /etc/xpm.conf (TOML format)

GENERAL OPTIONS:
    [options]
    root_dir = "/"                    # Installation root
    db_path = "/var/lib/xpm/"         # Database directory
    cache_dir = "/var/cache/xpm/pkg/" # Package cache
    log_file = "/var/log/xpm.log"     # Log file location
    gpg_dir = "/etc/xpm/gnupg/"       # Native xpm GPG keyring
    sig_level = "optional"            # required | optional | never
    parallel_downloads = 10           # Concurrent downloads
    check_space = true                # Check disk space
    color = true                      # Colored output
    architecture = "x86_64"           # System architecture
    alpm_hooks = true                 # false skips pacman-style .hook files

PACKAGE LISTS:
    hold_pkg = ["linux"]              # Never upgrade these
    ignore_pkg = ["pkg1", "pkg2"]     # Skip during upgrades
    ignore_group = ["group1"]         # Skip entire groups

REPOSITORY DEFINITION:
    [[repo]]
    name = "core"
    server = [
        "https://mirror.example.com/$repo/os/$arch",
        "https://mirror2.example.com/$repo/os/$arch"
    ]
    sig_level = "required"            # Override global setting

URL VARIABLES:
    $repo   Repository name (e.g., "core", "extra")
    $arch   System architecture (e.g., "x86_64")

FILES:
    /etc/xpm.conf           Main configuration
    /etc/xpm.d/*.toml       User-added repositories
"#
    );
}

pub(super) fn print_help_repos() {
    println!(
        r#"xpm — Repository Management

PREDEFINED REPOSITORIES:
    Configured in /etc/xpm.conf as [[repo]] sections.
    These are managed by the distribution maintainers.

USER-ADDED REPOSITORIES:
    Stored as individual files in /etc/xpm.d/
    Managed via `xpm repo` commands.

COMMANDS:
    xpm repo list                   List all repositories
    xpm repo add <name> <url>       Add a repository
    xpm repo remove <name>          Remove a repository

EXAMPLES:
    # Add Chaotic-AUR repository
    xpm repo add chaotic-aur https://cdn-mirror.chaotic.cx/$repo/$arch

    # Add a GitHub Pages hosted repo
    xpm repo add my-repo https://user.github.io/my-repo/$arch

    # Add a local file repository
    xpm repo add local file:///srv/packages/$arch

URL VARIABLES:
    $repo   Replaced with the repository name
    $arch   Replaced with system architecture (x86_64, aarch64)

SIGNATURE LEVELS:
    required    Signatures must be present and valid
    optional    Verify if present, allow unsigned (default)
    never       Skip verification completely

After adding a repository, run `xpm sync` to fetch its database.
"#
    );
}
