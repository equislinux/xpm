# Fetch Targets — Repositories, Mirrors, and Sync Endpoints

This document defines the repository structure, mirror configuration, and synchronization
endpoints used by xpm.

---

## Overview

xpm fetches packages and database files from configured repositories. Each repository
can have multiple mirrors for redundancy and performance.

```
Repository
├── Database files (.db, .files)
└── Package files (.xp or .pkg.tar.zst)
```

---

## URL Format and Variables

Repository URLs support variable substitution for dynamic path construction.

### Supported Variables

| Variable | Description | Example Value |
|----------|-------------|---------------|
| `$repo` | Repository name | `core`, `extra`, `x-repo` |
| `$arch` | System architecture | `x86_64`, `aarch64` |

### URL Examples

```
# Standard Arch Linux mirror
https://mirror.rackspace.com/archlinux/$repo/os/$arch

# GitHub Pages hosted repository  
https://equislinux.github.io/x-repo/$arch

# Local mirror
file:///srv/repo/$repo/$arch

# Custom CDN
https://cdn.example.com/packages/$repo/os/$arch
```

---

## Predefined Repositories

These repositories are built into xpm and configured in `/etc/xpm.conf`.

### X Distribution Default

| Repository | Description | Priority |
|------------|-------------|----------|
| `x-repo` | X Distribution packages | 1 (highest) |
| `core` | Arch Linux core system | 2 |
| `extra` | Arch Linux extra packages | 3 |

### Default Configuration

```toml
# /etc/xpm.conf

[[repo]]
name = "x-repo"
server = ["https://equislinux.github.io/x-repo/$arch"]
sig_level = "optional"

[[repo]]
name = "core"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://mirrors.kernel.org/archlinux/$repo/os/$arch"
]

[[repo]]
name = "extra"
server = [
    "https://mirror.rackspace.com/archlinux/$repo/os/$arch",
    "https://mirrors.kernel.org/archlinux/$repo/os/$arch"
]
```

---

## Sync Endpoints

### Database Files

Each repository provides these database files:

| File | Description | Required |
|------|-------------|----------|
| `<repo>.db` | Package metadata database | Yes |
| `<repo>.db.sig` | Database signature | If `sig_level != never` |
| `<repo>.files` | File listing database | Optional |
| `<repo>.files.sig` | Files database signature | Optional |

### Sync URL Construction

```
Base URL:     https://mirror.example.com/archlinux/$repo/os/$arch
Database:     https://mirror.example.com/archlinux/core/os/x86_64/core.db
Signature:    https://mirror.example.com/archlinux/core/os/x86_64/core.db.sig
Files DB:     https://mirror.example.com/archlinux/core/os/x86_64/core.files
```

### Package Download URLs

```
Base URL:     https://mirror.example.com/archlinux/$repo/os/$arch
Package:      https://mirror.example.com/archlinux/core/os/x86_64/linux-6.7.1-1-x86_64.pkg.tar.zst
Signature:    https://mirror.example.com/archlinux/core/os/x86_64/linux-6.7.1-1-x86_64.pkg.tar.zst.sig
```

---

## GitHub Pages Backend

xpm supports hosting repositories on GitHub Pages for easy distribution.

### Repository Structure

```
x-repo/
├── x86_64/
│   ├── x-repo.db
│   ├── x-repo.db.sig
│   ├── x-repo.files
│   ├── package-1.0.0-1-x86_64.xp
│   ├── package-1.0.0-1-x86_64.xp.sig
│   └── ...
└── aarch64/
    ├── x-repo.db
    └── ...
```

### GitHub Pages URL Format

```
https://<owner>.github.io/<repo>/$arch/<db-or-package>
```

### Adding GitHub Pages Repository

```bash
xpm repo add my-repo https://username.github.io/my-repo/$arch
```

---

## Mirror Selection

### Priority Order

1. User-added repositories (`/etc/xpm.d/*.toml`)
2. Predefined repositories (config order)
3. Within repository: first available mirror

### Fallback Behavior

```
Mirror 1 (primary)
    ↓ timeout/error
Mirror 2 (fallback)
    ↓ timeout/error
Mirror N
    ↓ all failed
Error: repository unreachable
```

### Future: Smart Mirror Selection

> **Planned for Phase 9** — Automatic mirror ranking based on:
> - Geographic proximity
> - Latency measurements
> - Bandwidth testing
> - Mirror sync status

---

## User-Added Repositories

Users can add temporary repositories stored in `/etc/xpm.d/`.

### Adding a Repository

```bash
xpm repo add chaotic-aur https://cdn-mirror.chaotic.cx/$repo/$arch
```

Creates `/etc/xpm.d/chaotic-aur.toml`:

```toml
name = "chaotic-aur"
server = ["https://cdn-mirror.chaotic.cx/$repo/$arch"]
```

### Listing Repositories

```bash
xpm repo list
```

Output:
```
:: Active repositories:

   [predefined]
   x-repo (1 server(s), sig: optional)
   core (2 server(s), sig: optional)
   extra (2 server(s), sig: optional)

   [user-added]
   chaotic-aur (1 server(s), sig: optional)
```

### Removing a Repository

```bash
xpm repo remove chaotic-aur
```

---

## Signature Verification Levels

| Level | Description |
|-------|-------------|
| `required` | Signatures must be present and valid |
| `optional` | Check if present, allow unsigned |
| `never` | Skip signature verification |

### Per-Repository Override

```toml
[[repo]]
name = "trusted-repo"
server = ["https://trusted.example.com/$repo/$arch"]
sig_level = "required"

[[repo]]
name = "local-testing"
server = ["file:///home/user/packages/$arch"]
sig_level = "never"
```

---

## Network Configuration

### Timeouts

| Operation | Default | Config Key |
|-----------|---------|------------|
| Connection | 10s | `connect_timeout` |
| Download | 300s | `download_timeout` |

### Parallel Downloads

```toml
[options]
parallel_downloads = 5    # Default: 5 concurrent downloads
```

### Proxy Support

```bash
# Environment variables
export http_proxy="http://proxy:8080"
export https_proxy="http://proxy:8080"
```

---

## Offline Mode

For air-gapped systems, use local file repositories:

```toml
[[repo]]
name = "offline-core"
server = ["file:///mnt/packages/core/$arch"]
sig_level = "never"
```

### Creating Local Mirror

```bash
# Sync a local mirror
rsync -avz rsync://mirror.example.com/archlinux/core/os/x86_64/ \
    /mnt/packages/core/x86_64/
```

---

## See Also

- [CLI Reference](CLI.md)
- [Configuration Example](../etc/xpm.conf.example)
- [ROADMAP Phase 5](../ROADMAP.md) — Repository database implementation
