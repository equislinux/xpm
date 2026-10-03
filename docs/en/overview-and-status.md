# xpm — Overview and Status

This document describes what `xpm` is, where it sits inside the X ecosystem, and its honest
current status within the reboot initiative. It is based on the repository contents as they are
today (README.md, ROADMAP.md, source code, tests, docs/), not on an intended future state.

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

The project belongs to the `xlnux` organization. Packages are built by the companion builder
tool `xpkg` (repo `xlnux/xpkg`); `xpm` consumes what `xpkg` produces.

## Position in the x-lnux workspace

The workspace `/home/x0z/Documents/repos/x-lnux` is not a single repository; each folder is an
independent GitHub repo with its own origin. The relevant folders are:

| Folder    | Repo             | Role |
|-----------|------------------|------|
| `x`       | `xlnux/x`        | The distro (archiso build of image/ISO and provisioning) |
| `scripts` | `xlnux/scripts`  | System kickstart / provisioning scripts |
| `xpkg`    | `xlnux/xpkg`     | Rust packaging tool (builder) |
| `xpm`     | `xlnux/xpm`      | Rust package manager (this repo) |
| `x-repo`  | `xlnux/x-repo`   | Package repository and portal |

This repo follows the shared conventions of the workspace: work for the reboot initiative lives
on the `x/reboot` branch of each repo, commits stay local (no push), and messages carry no emojis
or external-tool references.

## Current status (honest)

The repository exists, compiles, and carries a substantial test suite (unit tests across both
crates plus integration tests under `tests/`). Within the reboot initiative, however, `xpm` is
**de-prioritised**:

- The workspace-level ROADMAP marks **Phase 4 "Tooling Rust (xpm/xpkg)" as postponed**
  (`POSPUESTA`). By maintainer decision, xpm/xpkg is out of the current scope.
- Today the distro payload is packaged as the `x-scripts` package using a **PKGBUILD + makepkg**
  flow and is published to `x-repo` through the **pacman-compatible repository**. Consumption on
  installed systems goes through pacman, not through xpm.
- `xpm` is therefore **not yet the active path** in the reboot flow. It is a functioning
  tooling codebase with its own internal roadmap, waiting to be revisited when the native `.xp`
  repository or the SAT resolver is actually required.

The repository still publishes its own binaries as `.xp` packages in the xpm-native tree (see the
README for the key bootstrap and signature checklist), which is separate from the pacman path
used in production today. The two layouts must not be confused: the xpm-native endpoint is
`https://xlnux.github.io/x-repo/x/$arch`, not the pacman endpoint under `/repo/x86_64`.

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

The workspace `Cargo.toml` currently reports version `0.1.0`.

## About the command state

Not every subcommand is fully wired to engine logic yet. From `crates/xpm/src/main.rs`:

- `sync`, `install`, `remove`, `upgrade`, and `repo` dispatch to real transaction/download logic.
- `query` (including `--orphans`), `search`, `info`, and `files` read the local and sync
  databases; the remaining gap before the native path is wiring the resolver into
  install/upgrade.

See [Usage](usage.md) for the full reference and [Architecture](architecture.md) for the
implementation details, including the note that the CLI install path currently selects packages
by name from the synced database rather than through the SAT solver.
