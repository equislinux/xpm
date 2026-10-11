# xpm — Overview and Status

This document describes what `xpm` is, where it sits inside the X ecosystem, and its current
status in the toolchain. It is based on the repository contents as they are today (README.md,
ROADMAP.md, source code, tests, docs/), not on an intended future state.

---

## What xpm is

`xpm` is a modern, high-performance package manager written in pure Rust for the X distribution.
It is designed as a native Rust replacement for `pacman` and `libalpm`:

- Native `.xp` package format (X Package, a `tar.zst` archive) with `.PKGINFO`, `.BUILDINFO`, and
  `.MTREE` metadata.
- Compatibility with Arch Linux `.pkg.tar.zst` packages and ALPM-style repository databases
  (`.db` / `.files`).
- SAT-based dependency resolution powered by the `resolvo` crate.
- TOML configuration file at `/etc/xpm.conf`.
- OpenPGP detached signature verification with a configurable `sig_level`
  (`required` / `optional` / `never`).
- Repository management with predefined repositories plus user-added repositories under
  `/etc/xpm.d/`.

The project belongs to the `equislinux` organization. Packages are built by the companion builder
tool `xpkg` (repo `equislinux/xpkg`); `xpm` consumes what `xpkg` produces.

## Where xpm sits

Each component lives in its own GitHub repository under `equislinux`:

| Folder    | Repo             | Role |
|-----------|------------------|------|
| `x`       | `equislinux/x`        | The distro (archiso build of image/ISO and provisioning) |
| `scripts` | `equislinux/scripts`  | System provisioning payload and the `x` CLI |
| `xpkg`    | `equislinux/xpkg`     | Rust packaging tool (builder) |
| `xpm`     | `equislinux/xpm`      | Rust package manager (this repo) |
| `x-repo`  | `equislinux/x-repo`   | Package repositories (pacman `[x]` and native `.xp`) and portals |

## Current status

`xpm` is wired end to end, and the native `.xp` endpoint it serves is live:

- The SAT resolver runs inside `install` and `upgrade`: dependency closure, dependency order,
  `name=version` requirements and unversioned `provides`.
- Local `.xp` installs, package removal and dependency-aware upgrades work through the same
  transaction engine; `sync`/`search` read the local and sync databases.
- The endpoint at `https://equislinux.github.io/x-repo/x/$arch` (built by `x-repo`'s
  `scripts/build-xp.sh`, separate from the pacman endpoint under `/repo/x86_64`) was validated
  with `xpm sync/search/deps/install` against the published URL.
- The system itself is still provisioned with **pacman + the signed `[x]` repository** —
  `xpm` coexists as the native channel rather than replacing the production flow.

Known open issues (tracked in this repo):

1. Resolver: `Unsolvable(Conflict)` for packages that depend on `gcc-libs` (dependency cycle)
   even on a clean root.
2. Extraction: hardlink extraction fails on some real Arch packages (`zoneinfo` files under
   `usr/share/zoneinfo/`).

## Internal roadmap and milestones

`xpm` keeps its own ROADMAP.md inside this repo with phases 0-10. Progress highlights:

- Phases 0-1 complete: scaffolding, CLI with 8 subcommands plus `repo`/`usage`, TOML config.
- Phase 2 (libalpm FFI bridge) skipped by design — the project went straight to native Rust to
  avoid C dependencies.
- Phase 3 complete: SAT resolver using `resolvo`, ALPM `vercmp` version comparison, dependency
  parsing, conflict handling, integration tests.
- Phase 4 complete: `.xp` / `.pkg.tar.zst` readers, metadata parsers, archive extraction, and
  post-install integrity validation.
- Phase 5 largely complete: ALPM `.db` / `.files` parsing, local database under
  `/var/lib/xpm/local/`, remote sync with HTTP downloads, GitHub Pages backend, URL variable
  substitution.
- Phase 10 security execution track: detached `.db.sig` and package `.sig` verification plus
  keyring loading are implemented.

The versioning convention in the repo roadmap is:

| Version  | Milestone |
|----------|-----------|
| `v0.1.0` | Phases 0-1 complete (functional CLI with configuration) |
| `v0.2.0` | Phase 2 skipped (no libalpm dependency) |
| `v0.5.0` | Phases 3-5 complete (native engine operational) |
| `v0.8.0` | Phases 6-7 complete (security + transactions) |
| `v1.0.0` | Phase 8 complete (benchmarked, tested, production-ready) |

The `Cargo.toml` currently reports version `0.1.0`.

## About the command state

Not every subcommand is fully wired to engine logic yet. From `crates/xpm/src/main.rs`:

- `sync`, `install`, `remove`, `upgrade`, and `repo` dispatch to real transaction/download logic.
- `install` runs the SAT resolver over the sync databases: dependency closure, dependency order,
  `name=version` requirements and unversioned `provides`.
- `query` (including `--orphans`), `search`, `info`, and `files` read the local and sync
  databases.
- `history` links each transaction to the generation it produced (`gen:NNNN`), `diff <generation>`
  compares the live database against a generation's `packages.tsv`, and `rollback --last`
  replays the inverse of the newest successful transaction from the package cache
  (`--dry-run` supported).
- `.pacnew`/`.pacsave` handling follows the `backup` entries of `.PKGINFO`: modified
  configuration files are never silently overwritten and survive removals unless `--nosave`.
- Pacman-style ALPM hooks (`/usr/share/libalpm/hooks`, `/etc/pacman.d/hooks`) run around every
  transaction, so the distribution's generation hooks work under xpm as well.

See [Usage](usage.md) for the full reference and [Architecture](architecture.md) for the
implementation details.
