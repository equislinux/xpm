# xpm CLI Reference

Complete reference for all xpm commands, flags, aliases, and invocation patterns.

---

## Global Flags

These flags can be used with any subcommand.

| Flag | Short | Value | Description |
|------|-------|-------|-------------|
| `--config` | `-c` | `PATH` | Path to configuration file (default: `/etc/xpm.conf`) |
| `--verbose` | `-v` | — | Increase verbosity (`-v`, `-vv`, `-vvv`) |
| `--no-confirm` | — | — | Suppress confirmation prompts |
| `--root` | — | `PATH` | Alternative installation root directory |
| `--dbpath` | — | `PATH` | Alternative database directory |
| `--cachedir` | — | `PATH` | Alternative cache directory |
| `--no-color` | — | — | Disable colored output |

---

## Commands

### `sync` — Synchronize Package Databases

Refresh package databases from configured repositories.

```bash
xpm sync [OPTIONS]
xpm Sy [OPTIONS]          # pacman-style alias
```

| Flag | Short | Description |
|------|-------|-------------|
| `--force` | `-f` | Force full refresh even if databases are up to date |

**Examples:**

```bash
xpm sync                  # Sync all databases
xpm sync --force          # Force full refresh
xpm Sy -f                 # Same as above, pacman-style
```

---

### `install` — Install Packages

Install one or more packages from sync databases.

```bash
xpm install <PACKAGES>... [OPTIONS]
xpm S <PACKAGES>... [OPTIONS]    # pacman-style alias
```

| Flag | Short | Description |
|------|-------|-------------|
| `--download-only` | `-w` | Download packages without installing |
| `--as-deps` | — | Mark packages as dependencies |
| `--as-explicit` | — | Mark packages as explicitly installed |
| `--no-optional` | — | Skip optional dependencies |

**Examples:**

```bash
xpm install firefox               # Install single package
xpm install vim neovim tmux       # Install multiple packages
xpm S -w linux linux-headers      # Download only
xpm install --as-deps libfoo      # Install as dependency
```

---

### `remove` — Remove Packages

Remove installed packages from the system.

```bash
xpm remove <PACKAGES>... [OPTIONS]
xpm R <PACKAGES>... [OPTIONS]    # pacman-style alias
```

| Flag | Short | Description |
|------|-------|-------------|
| `--recursive` | `-s` | Also remove unneeded dependencies |
| `--no-deps` | `-d` | Skip dependency checks |
| `--nosave` | `-n` | Remove configuration files (purge) |

**Examples:**

```bash
xpm remove firefox                # Remove single package
xpm R -s vim                      # Remove with unused deps
xpm remove -n --recursive pkg     # Purge with deps
```

---

### `upgrade` — System Upgrade

Upgrade all installed packages to their latest versions.

```bash
xpm upgrade [OPTIONS]
xpm Su [OPTIONS]          # pacman-style alias
```

| Flag | Short | Description |
|------|-------|-------------|
| `--force` | — | Force reinstall of up-to-date packages |
| `--ignore` | — | Skip specific packages (repeatable) |

**Examples:**

```bash
xpm upgrade                       # Full system upgrade
xpm Su --ignore linux             # Upgrade, skip linux
xpm upgrade --ignore pkg1 --ignore pkg2
```

---

### `history` — Transaction Journal

Show the recorded transactions, newest first. Every `install`, `remove` and
`upgrade` writes a JSON entry under `<db_path>/journal/<epoch>-<pid>.json`
before touching the filesystem and finalizes it as `ok`/`failed` after the
commit. `pre-transaction.d`/`post-transaction.d` hooks (default
`/usr/lib/xpm/hooks`, override with `XPM_HOOKS_DIR`) run around it with
`XPM_ROOT_DIR`, `XPM_ACTION`, `XPM_JOURNAL`, `XPM_PKG_NAMES` and
`XPM_PKG_VERSIONS`. See `GENERATIONS.md`.

```bash
xpm history [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `--json` | Emit one JSON object per transaction (machine consumption) |

**Examples:**

```bash
xpm history                # Human summary (ISO-8601 timestamps)
xpm history --json         # One JSON line per transaction
```

Transactions left in `running` state (for example after a crash) stay in the
journal as evidence; the generation layer (`x gen`) is the recovery path.

---

### `query` — Query Local Database

Query the local package database for installed packages.

```bash
xpm query [FILTER] [OPTIONS]
xpm Q [FILTER] [OPTIONS]         # pacman-style alias
```

| Flag | Short | Description |
|------|-------|-------------|
| `--format` | — | Output format: `plain` (default) or `tsv` |
| `--explicit` | `-e` | List only explicitly installed packages |
| `--deps` | `-d` | List only packages installed as dependencies |
| `--orphans` | `-t` | List orphan packages (no longer required) |
| `--upgrades` | `-u` | List outdated packages |

**Positional Arguments:**

| Argument | Description |
|----------|-------------|
| `FILTER` | Optional package name filter |

**Examples:**

```bash
xpm query                         # List all installed packages
xpm Q -e                          # List explicit packages only
xpm query --orphans               # Find orphan packages
xpm Q -u                          # List upgradeable packages
xpm query vim                     # Filter by name
```

---

### `search` — Search Packages

Search for packages in sync or local databases.

```bash
xpm search <QUERY> [OPTIONS]
xpm Ss <QUERY> [OPTIONS]         # pacman-style alias
```

| Flag | Short | Description |
|------|-------|-------------|
| `--local` | `-l` | Search in local database instead of sync |

**Examples:**

```bash
xpm search firefox                # Search sync database
xpm Ss "text editor"              # Search by description
xpm search --local vim            # Search installed packages
```

---

### `info` — Package Information

Display detailed information about an installed package: name, version,
install reason (`explicit`/`dep`) and origin repository. When the sync
database is available, the repository description and dependencies are
added (highest-priority repository wins, same order as `xpm query
--upgrades`). For packages that are not installed, the sync entry alone is
shown.

```bash
xpm info <PACKAGE> [OPTIONS]
xpm Si <PACKAGE> [OPTIONS]       # pacman-style alias
```

| Flag | Short | Description |
|------|-------|-------------|
| `--local` | `-l` | Query the local database only (no sync enrichment) |

**Examples:**

```bash
xpm info linux                    # Local info + sync description/deps
xpm Si firefox                    # Same, pacman-style
xpm info --local vim              # Installed metadata only
```

**Output (installed package):**

```
Name            : kitty
Version         : 0.44.0-1
Install Reason  : explicit
Origin          : x
Repository      : x
Description     : A cross-platform, fast, feature full, GPU based terminal emulator
Depends On      : glfw  libglvnd  wayland
```

Legacy installs without `reason`/`origin` files default to `explicit` and
`unknown` instead of failing.

---

### `files` — List Package Files

List all files owned by an installed package, read from
`<db_path>/local/<pkg>/files`. The manifest is pacman-compatible: it is
derived from the package's `.MTREE`, so it includes directories (trailing
`/`) and symlinks. This is the same manifest consumed by
`x gen restore --pkg`.

```bash
xpm files <PACKAGE>
xpm Ql <PACKAGE>                 # pacman-style alias
```

**Examples:**

```bash
xpm files bash                    # List files in bash package
xpm Ql linux                      # pacman-style
```

**Output:**

```
usr/
usr/bin/
usr/bin/bash
```

---

### `repo` — Repository Management

Manage package repositories (add, remove, list).

```bash
xpm repo <ACTION>
```

#### `repo list`

List all active repositories (predefined + user-added).

```bash
xpm repo list
```

#### `repo add`

Add a temporary user repository.

```bash
xpm repo add <NAME> <URL>
```

| Argument | Description |
|----------|-------------|
| `NAME` | Repository identifier (e.g., `my-repo`) |
| `URL` | Mirror URL (e.g., `https://example.com/repo/os/x86_64`) |

**Examples:**

```bash
xpm repo add chaotic-aur https://cdn-mirror.chaotic.cx/$repo/$arch
xpm repo add x-repo https://xlnux.github.io/x-repo/$arch
```

#### `repo remove`

Remove a user-added repository.

```bash
xpm repo remove <NAME>
```

---

### `usage` — Detailed Help

Display detailed help information.

```bash
xpm usage [TOPIC]
```

| Topic | Description |
|-------|-------------|
| (none) | General overview and quick reference |
| `commands` | List all available commands |
| `config` | Configuration file format and options |
| `repos` | Repository configuration and management |
| `<command>` | Detailed help for a specific command |

**Examples:**

```bash
xpm usage                         # General help
xpm usage install                 # Help for install command
xpm usage config                  # Configuration help
xpm usage repos                   # Repository help
```

---

## Pacman Compatibility Matrix

| xpm Command | Pacman Equivalent | Description |
|-------------|-------------------|-------------|
| `xpm sync` | `pacman -Sy` | Sync databases |
| `xpm Sy` | `pacman -Sy` | Alias |
| `xpm install <pkg>` | `pacman -S <pkg>` | Install package |
| `xpm S <pkg>` | `pacman -S <pkg>` | Alias |
| `xpm remove <pkg>` | `pacman -R <pkg>` | Remove package |
| `xpm R <pkg>` | `pacman -R <pkg>` | Alias |
| `xpm upgrade` | `pacman -Su` | System upgrade |
| `xpm Su` | `pacman -Su` | Alias |
| `xpm query` | `pacman -Q` | Query local DB |
| `xpm Q` | `pacman -Q` | Alias |
| `xpm search <q>` | `pacman -Ss <q>` | Search sync DB |
| `xpm Ss <q>` | `pacman -Ss <q>` | Alias |
| `xpm info <pkg>` | `pacman -Si <pkg>` | Package info |
| `xpm Si <pkg>` | `pacman -Si <pkg>` | Alias |
| `xpm files <pkg>` | `pacman -Ql <pkg>` | List files |
| `xpm Ql <pkg>` | `pacman -Ql <pkg>` | Alias |

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `XPM_CONFIG` | Override default configuration file path |
| `XPM_CACHE_DIR` | Override default cache directory |
| `XPM_HOOKS_DIR` | Override the transaction-hook root (default `/usr/lib/xpm/hooks`) |
| `NO_COLOR` | Disable colored output (standard) |
| `RUST_LOG` | Set logging verbosity (e.g., `debug`, `trace`) |

---

## Exit Codes

| Code | Description |
|------|-------------|
| `0` | Success |
| `1` | General error |
| `2` | Invalid arguments or usage |
| `3` | Package not found |
| `4` | Dependency resolution failed |
| `5` | Transaction aborted by user |
| `6` | Permission denied |
| `7` | Database locked |

---

## See Also

- [Configuration Reference](../etc/xpm.conf.example)
- [Fetch Targets](FETCH_TARGETS.md)
- [ROADMAP](../ROADMAP.md)
