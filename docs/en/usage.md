# xpm — Usage

Complete usage reference for the `xpm` binary, based on `crates/xpm/src/cli/`,
`crates/xpm/src/commands/`, and the project README.

---

## Global flags

These flags are accepted by every subcommand.

| Flag | Short | Value | Description |
|------|-------|-------|-------------|
| `--config` | `-c` | `PATH` | Path to the configuration file (default `/etc/xpm.conf`) |
| `--verbose` | `-v` | count | Increase verbosity (`-v`, `-vv`, `-vvv`) |
| `--no-confirm` | | | Skip confirmation prompts |
| `--root` | | `PATH` | Alternative installation root directory |
| `--dbpath` | | `PATH` | Alternative database directory |
| `--cachedir` | | `PATH` | Alternative cache directory |
| `--no-color` | | | Disable colored output |
| `--no-alpm-hooks` | | | Skip pacman-style ALPM `.hook` execution for this transaction |

The CLI is defined with `clap` and requires a subcommand (`arg_required_else_help = true`).

## Commands

### `sync` — Synchronize package databases

Alias: `Sy`. Downloads the latest `.db` (and best-effort `.files`) database files from every
configured repository and parses them into local sync databases.

```bash
xpm sync [OPTIONS]
xpm Sy [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--force` | `-f` | Force a full database refresh even if databases look up to date |

Implementation note: `cmd_sync` in `commands/sync.rs` runs per-repository sync workers in parallel chunks,
tries each configured mirror with retries, reports which mirror answered, and then parses the
downloaded `.db` / `.files` so the local package count can be shown. Remote sync failures are
warned and do not abort the whole run.

### `install` — Install packages

Alias: `S`. Installs one or more packages by name from the synchronized databases, or
directly from a local `.xp` / `.pkg.tar.*` archive.

```bash
xpm install <PACKAGES>... [OPTIONS]
xpm install /path/to/package.pkg.tar.zst
xpm S <PACKAGES>... [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--download-only` | `-w` | Download packages without installing |
| `--as-deps` | | Mark the package as installed as a dependency |
| `--as-explicit` | | Mark the package as explicitly installed |
| `--no-optional` | | Skip optional dependencies |

Behavior (from `commands/install/`): every configured sync database is loaded and the requested
requirements (`name` or `name=version`) are solved with the SAT resolver, which picks candidates,
honors `depends`/`conflicts` and unversioned `provides`, and returns the closure in dependency
order. Each package is downloaded to the cache directory, checked against a remote `.sig` per the
effective `sig_level` and against `sha256sum` when the database entry carries one, then committed
as install operations on a `Transaction` (requested packages explicit, pulled dependencies as
deps; `--as-deps`/`--as-explicit` override). With `--download-only` the run stops after
downloading. Otherwise xpm asks for confirmation (unless `--no-confirm`) and the transaction
extracts the files and registers each package in the local database.

### `remove` — Remove packages

Alias: `R`. Removes installed packages, using the file manifest recorded in the local database.

```bash
xpm remove <PACKAGES>... [OPTIONS]
xpm R <PACKAGES>... [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--recursive` | `-s` | Also remove unneeded dependencies |
| `--no-deps` | `-d` | Skip dependency checks |
| `--nosave` | `-n` | Remove configuration files as well (purge) |

The package must be registered in the local database (otherwise xpm reports it is not
installed). Confirmation is requested unless `--no-confirm`.

### `upgrade` — System upgrade

Alias: `Su`. Upgrades all installed packages to the newest available versions.

```bash
xpm upgrade [OPTIONS]
xpm Su [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--force` | | Force reinstallation of up-to-date packages |
| `--ignore` | | Skip specific packages (repeatable, `--ignore <PKG>`) |

`upgrade` always refreshes the databases first (equivalent to `pacman -Syu`), then resolves the
transitive closure of the packages with newer versions so new or newly-required dependencies are
installed in the same run. Upgraded packages keep their install reason; pulled dependencies are
recorded as deps. With no packages installed it reports "Nothing to do".

### `query` — Query the local database

Alias: `Q`. Lists installed packages from the local database.

```bash
xpm query [FILTER] [OPTIONS]
xpm Q [FILTER] [OPTIONS]
```

| Argument / Flag | Short | Description |
|-----------------|-------|-------------|
| `FILTER` | | Optional package name filter |
| `--explicit` | `-e` | Only explicitly installed packages |
| `--deps` | `-d` | Only packages installed as dependencies |
| `--orphans` | `-t` | Orphan packages (no longer required) |
| `--upgrades` | `-u` | Packages with available updates |

Implementation note: implemented against the local and sync databases (name filter plus
`--explicit`, `--deps` and `--upgrades`). `--orphans` lists dependency packages that no
explicitly installed package requires, using the `depends`/`provides` edges recorded at install
time; legacy entries without a record are skipped.

### `search` — Search packages

Alias: `Ss`. Searches for packages by name, description, or provides.

```bash
xpm search <QUERY> [OPTIONS]
xpm Ss <QUERY> [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--local` | `-l` | Search in the local database instead of the sync databases |

Implementation note: implemented as a case-insensitive match over name, description and
provides; `--local` matches installed package names.

### `info` — Package information

Alias: `Si`. Displays detailed information about a package.

```bash
xpm info <PACKAGE> [OPTIONS]
xpm Si <PACKAGE> [OPTIONS]
```

| Flag | Short | Description |
|------|-------|-------------|
| `--local` | `-l` | Query the local database instead of the sync databases |

Implementation note: implemented; merges the installed entry with the highest-priority sync
entry, and `--local` limits the output to the installed database.

### `files` — List package files

Alias: `Ql`. Lists all files owned by an installed package.

```bash
xpm files <PACKAGE>
xpm Ql <PACKAGE>
```

Implementation note: implemented; reads the `files` manifest recorded at install time (empty
for legacy installs).

### `history`, `rollback`, `diff` — Journal and generations

- `xpm history [--json]` — recorded transactions, newest first. Finished entries link the
  generation they produced (`gen:NNNN`, when the state directory is readable).
- `xpm rollback [--last | --journal <ID>] [--dry-run]` — replay the inverse of a successful
  transaction using the package cache; aborts before changing anything if an old package file
  is missing.
- `xpm diff <GENERATION> [--json]` — compare the installed packages against a generation's
  `packages.tsv` (`current` resolves the default id).

Configuration files declared with `backup` in `.PKGINFO` follow pacman semantics: `.pacnew` on
install/upgrade and `.pacsave` on removal (disable with `--nosave`). Pacman-style ALPM hooks
from `/usr/share/libalpm/hooks` and `/etc/pacman.d/hooks` run around every transaction.

### `repo` — Repository management

Manages user-added (temporary) repositories. Predefined repositories come from `/etc/xpm.conf`;
user-added repositories are stored as TOML files under `/etc/xpm.d/`.

```bash
xpm repo list                 # predefined + user-added repositories
xpm repo add <NAME> <URL>     # add a temporary repository
xpm repo remove <NAME>        # remove a user-added repository
```

Examples (from the built-in help):

```bash
xpm repo add chaotic-aur https://cdn-mirror.chaotic.cx/$repo/$arch
xpm repo add my-repo https://username.github.io/my-repo/$arch
xpm repo add local file:///srv/packages/$arch
```

`repo add` refuses to overwrite an existing entry of the same name. After adding a repository,
run `xpm sync` to fetch its database.

### Configuration file and mirrors

Predefined repositories are defined in `/etc/xpm.conf` (TOML). Each `[[repo]]` block is one
logical repository, and its `server` key is an ordered list of mirrors for that same
repository:

```toml
[[repo]]
name = "core"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
    "https://mirror.osbeck.com/archlinux/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"
```

`$repo` and `$arch` are expanded at fetch time. xpm tries the mirrors in the listed order and
uses the first one that serves the database; package downloads use the same list. The order is
a preference and failover order, not a latency measurement, so put the mirror that is most
reliable for your network first.

Rules to keep in mind:

- Declare each repository once. Two `[[repo]]` blocks with the same `name` write the same
  `<name>.db` file and conflict; to add mirrors, extend the existing `server` list.
- `xpm repo add <NAME> <URL>` adds a different repository (stored in `/etc/xpm.d/<name>.toml`),
  it does not add a mirror to an existing one, and it refuses to overwrite an existing name.
- Repository order is package priority: the first repository that provides a package wins.
  Testing and unstable repositories must be listed before their stable counterparts to take
  effect; staging repositories are for packaging work.
- `/etc/xpm.conf` is a plain configuration file and is not owned by a package. Edits persist
  across reboots and package upgrades. Reinstalling X regenerates the file with the installer
  default (`X_XPM_ARCH_MIRROR`, which defaults to `geo.mirror.pkgbuild.com`).
- After editing the file, run `sudo xpm sync --force`. `xpm clean` removes cached package files
  only; it does not reset sync databases. If a sync fails and an old database remains, packages
  from it can return 404 until the next successful sync.

Mirror coverage for the official Arch repositories:

| Mirror | Repositories | Notes |
|--------|--------------|-------|
| `https://mirror.rackspace.com/archlinux/$repo/os/$arch` | all 14 | Includes `*-debug` |
| `https://geo.mirror.pkgbuild.com/$repo/os/$arch` | all 14 | Official GeoDNS; speed varies |
| `https://mirror.osbeck.com/archlinux/$repo/os/$arch` | all except `*-debug` | Debug requests return 404 |

#### Copy-paste: X plus every official Arch repository

The file below is a complete `/etc/xpm.conf`. It keeps the signed `[x]` repository first and
adds every official Arch repository: stable, testing, staging, debug, and unstable. Testing,
staging, and unstable repositories move quickly, so enable only the blocks you need. The
example follows the x86_64 repository layout; other architectures use the repositories
published for that architecture.

```toml
[options]
root_dir = "/"
db_path = "/var/lib/xpm"
cache_dir = "/var/cache/xpm/pkg"
log_file = "/var/log/xpm.log"
gpg_dir = "/etc/xpm/gnupg"
sig_level = "required"
database_sig_level = "optional"
parallel_downloads = 5

[[repo]]
name = "x"
server = ["https://equislinux.github.io/x-repo/x/$arch"]
sig_level = "required"
database_sig_level = "required"

# Testing and unstable repositories must come before their stable counterparts
# to take effect: the first repository that provides a package wins.
[[repo]]
name = "core-testing"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "extra-testing"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "multilib-testing"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "kde-unstable"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "gnome-unstable"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "core"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "extra"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "multilib"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

# Staging repositories are placed after stable on purpose. They are for
# packaging work; move them before stable only if you are preparing packages.
[[repo]]
name = "core-staging"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "extra-staging"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "multilib-staging"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

# Debug repositories contain packages with the -debug suffix; their order
# relative to the other repositories does not matter.
[[repo]]
name = "core-debug"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "extra-debug"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "multilib-debug"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"
```

#### Variant: prefer osbeck (not available for `*-debug`)

`mirror.osbeck.com` is fast from many networks but does not mirror the debug repositories. To
prefer it, put it first in every block except `core-debug`, `extra-debug`, and
`multilib-debug`:

```toml
[[repo]]
name = "core"
server = [
    "https://mirror.osbeck.com/archlinux/$repo/os/$arch",
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"

[[repo]]
name = "core-debug"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://geo.mirror.pkgbuild.com/$repo/os/$arch",
]
sig_level = "required"
database_sig_level = "optional"
```

Repeat the same pattern for the remaining repositories.

### `usage` — Built-in help

Shows detailed usage help for the whole tool or for a topic/command.

```bash
xpm usage                    # general overview
xpm usage commands           # list all commands
xpm usage config             # configuration file format
xpm usage repos              # repository management
xpm usage <command>          # help for a specific command (sync, install, remove, upgrade, ...)
```

`xpm <command> --help` also works through clap.

## Progress bars

Progress is split by phase: downloads use `x`, while verification, install,
upgrade and removal use `/`. The combined
`:: [bar] download | verify | extract` line is gone.

## Pacman-style aliases

| Alias | Maps to | Pacman equivalent |
|-------|---------|-------------------|
| `Sy` | `sync` | `pacman -Sy` |
| `S` | `install` | `pacman -S` |
| `R` | `remove` | `pacman -R` |
| `Su` | `upgrade` | `pacman -Su` |
| `Q` | `query` | `pacman -Q` |
| `Ss` | `search` | `pacman -Ss` |
| `Si` | `info` | `pacman -Si` |
| `Ql` | `files` | `pacman -Ql` |

## Typical workflow

```bash
xpm sync                       # refresh package databases
xpm install <package>          # install a package
xpm upgrade                    # upgrade installed packages (syncs first)
xpm query                      # list installed packages
xpm remove <package>           # remove a package
```

Non-interactive use (for scripts) needs `--no-confirm`. For isolated/rootless experiments use
`--config`, `--root`, `--dbpath`, and `--cachedir` to point at temporary directories; when the
installation root is not `/`, xpm enables shell integration and creates command shims in
`~/.local/bin` (with PATH export lines in `~/.bashrc` and `~/.zshrc`).

## Environment variables and exit codes

`RUST_LOG` is honoured through `tracing-subscriber`'s `EnvFilter` to control log verbosity.
`docs/CLI.md` additionally documents `XPM_CONFIG`, `XPM_CACHE_DIR`, `XPM_HOOKS_DIR`,
`XPM_ALPM_HOOKS_DIRS`, `X_GEN_STATE`, and `NO_COLOR`.

`docs/CLI.md` documents an exit-code matrix (0 success, 1 general error, 2 usage error, up to 7
database locked). Note that this matrix is documented intent rather than an enforced contract in
the current code: in practice clap reports usage errors, `anyhow` reports runtime failures, and
other documented codes are not yet emitted by the command handlers. Verify against the code before relying
on a specific code.

## References

- Existing CLI reference: [`../CLI.md`](../CLI.md)
- Fetch targets and mirror layout: [`../FETCH_TARGETS.md`](../FETCH_TARGETS.md)
- Install/upgrade quick guide: [`../INSTALL_AND_UPGRADE.md`](../INSTALL_AND_UPGRADE.md)
- Example configuration: [`../../etc/xpm.conf.example`](../../etc/xpm.conf.example)
- Command definition: [`../../crates/xpm/src/cli/`](../../crates/xpm/src/cli/),
  dispatch: [`../../crates/xpm/src/commands/mod.rs`](../../crates/xpm/src/commands/mod.rs)
