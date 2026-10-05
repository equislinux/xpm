# Roadmap — xpm Package Manager

> Rust-based package manager for the 'X' distribution (Arch Linux-based).

## Current Status

The install path is complete end to end: every sync database is loaded, the
requested requirements (`name` or `name=version`) are solved with the SAT
resolver (dependency closure, dependency order, conflicts and unversioned
`provides`), packages are downloaded and signature/checksum-verified, then
committed as transactions that record `reason`, `origin`, `files`, `depends`
and `provides` in the local database. `query` (including `--orphans`),
`search`, `info`, `files`, `remove`, `upgrade`, `repo`, `history` and `usage`
are wired to real engine logic. Test suite: ~171 unit tests plus the
repository-database integration tests and the xpkg↔xpm lifecycle E2E.

Still missing before the native path replaces pacman: local `.xp` file
install, dependency-aware `upgrade` closures, `rollback --last`,
`diff <generation>`, `.pacnew`/`.pacsave` handling and benchmark coverage.

---

## Phase 0 · Project Scaffolding <!-- phase:phase-0:scaffolding -->

- [x] Initialize Rust crate with cargo init (#1)
- [x] Configure Cargo workspace — multi-crate (#2)
- [x] Add linter and formatter configuration (#3)
- [x] Set up CI pipeline (#4)
- [x] Add license and crate metadata (#5)
- [x] Update README to reflect current architecture and repo strategy (#48)

## Phase 1 · CLI and Configuration <!-- phase:phase-1:cli -->

- [x] Implement CLI interface with clap (#6)
- [x] Implement configuration parser (#7)
- [x] Implement main.rs orchestration (#8)
- [x] Integrate lib.rs with CLI (#9)
- [x] Define activation commands and parameter matrix — document all CLI invocations, flags and aliases (#46)
- [x] Define fetch targets — repositories, mirrors and sync endpoints (#47)
- [x] Implement repo subcommand — xpm repo add, remove, list (#49)
- [x] Implement temporary repo file — /etc/xpm.d/ directory for user-added repos (#50)
- [x] Set predefined default repo in config — GitHub Pages x-repo as built-in (#51)
- [x] Implement custom help parameter — xpm help with detailed usage info (#54)

## Phase 2 · FFI Bindings with libalpm — Transitional Bridge <!-- phase:phase-2:ffi-bridge -->

> **Skipped** — going directly to native Rust implementation to avoid C dependencies.

- [x] Integrate alpm.rs crate (#10)
- [x] Create Rust wrapper over ALPM operations (#11)
- [x] Integration tests with local repository (#12)

## Phase 3 · Native Dependency Resolution Engine <!-- phase:phase-3:resolver -->

- [x] Integrate resolvo SAT solver (#13)
  - [x] Add resolvo crate and implement DependencyProvider trait
  - [x] Implement version parser and comparator — full vercmp compatible with ALPM versioning
  - [x] Implement dependency string parser — parse operators >=, <=, =, >, < from dep strings
  - [x] Implement core resolver types — Package, Dependency, Conflict, VersionReq structs
- [x] Implement dependency-to-CNF clause translator (#14)
- [x] Implement Unit Propagation with watched literals (#15)
- [x] Implement CDCL — Conflict-Driven Clause Learning (#16)
- [x] Write dependency resolution test suite (#17)
  - [x] Unit tests for version comparison and dependency parsing
  - [x] Integration tests for full solve scenarios

## Phase 4 · Package Format and Archives <!-- phase:phase-4:packages -->

- [x] Implement .pkg.tar.zst parser and builder (#18)
  - [x] Implement .pkg.tar.zst archive reader — zstd decompression + tar extraction
  - [x] Implement .pkg.tar.zst builder — create packages from directory tree
- [x] Define .xp package format — X Package native format (#59)
  - [x] Document .xp specification — tar.zst archive with .PKGINFO, .BUILDINFO, .MTREE
  - [x] Implement .xp archive reader — zstd decompression + tar extraction
  - [x] Support dual extensions — .xp (X Distribution) and .pkg.tar.zst (Arch compatibility)
- [x] Create xpkg tool — separate package building from xpm into dedicated repository (#60)
- [x] Implement package metadata parser (#19)
  - [x] Implement .PKGINFO parser — extract name, version, dependencies, provides, conflicts
  - [x] Implement .BUILDINFO parser — reproducible build environment metadata
  - [x] Implement .MTREE parser — file integrity hashes and permissions
  - [x] Implement PackageMeta types — unified structs for all package metadata fields
- [x] Implement post-installation integrity validation (#20)
  - [x] Verify extracted files against .MTREE checksums
- [x] Write package format tests (#21)
  - [x] Round-trip tests — build and re-parse .xp packages
  - [ ] Parse real Arch Linux .pkg.tar.zst packages for compatibility

## Phase 5 · Repository Database <!-- phase:phase-5:repo-db -->

- [x] Implement alpm-repo-db parser (#22)
  - [x] Read desc and depends entries from repo .db tar archives
  - [x] Implement repo database types — RepoEntry, SyncDb, LocalDb structs
- [x] Implement alpm-repo-files support (#23)
  - [x] Parse file listings from .files archives
- [ ] Implement agnostic symlink handling (#24)
  - [x] Implement local package database — track installed packages under /var/lib/xpm/local/
- [x] Implement remote database sync (#25)
  - [x] Implement HTTP download client — reqwest wrapper with progress, retries and parallel downloads
  - [x] Download and update .db files from configured mirrors
- [x] Implement GitHub Pages repo backend — fetch packages from static hosting (#52)
- [x] Implement repo URL variable substitution — $repo, $arch placeholders (#53)
- [x] Write repository database test suite — parser tests with real Arch .db files (#56)
  - [x] Unit tests for Arch-style .db parsing (no FILENAME/SHA256SUM/URL fields)
  - [x] Unit tests for xpkg-extended .db format (with fetch metadata)
  - [x] Integration tests using real x.db.tar.gz from x-repo repository
  - [x] Tests for complex dependency structures and version formats
  - [x] Tests for large repository databases (stress testing)

## Phase 6 · Security and Verification <!-- phase:phase-6:security -->

- [x] Implement OpenPGP signature verification (#26)
  - [x] Verify detached .sig files via sequoia-openpgp (pure Rust)
- [ ] Implement Berblom algorithm for key management (#27)
  - [ ] Import, list, trust and revoke keys in local keyring
- [ ] Implement Web of Trust — WoT — model (#28)
- [ ] Implement fakeroot build environment (#29)
- [ ] Implement package linting framework (#30)
- [ ] Write security test suite — signature verification, key management and linting tests (#57)

## Phase 7 · Transactions and System Management <!-- phase:phase-7:transactions -->

- [x] Implement transaction engine (#31)
  - [x] Plan, prepare and commit install/remove/upgrade operations
- [/] Implement pre/post transaction hooks (#32)
  - [x] Hook trait and HookChain infrastructure
  - [x] Local database registration (install/remove)
  - [x] File removal hook (for clean uninstall)
  - [x] File extraction — install package files to filesystem with correct ownership
  - [ ] Execute scriptlets and alpm-hooks
- [ ] Implement configuration file management (#33)
  - [ ] Handle .pacnew and .pacsave generation
- [x] Implement database lock mechanism (#34)
  - [x] Prevent concurrent xpm operations via FileLock
- [x] Implement transaction logging (#35)
  - [x] Append operations to /var/log/xpm.log
- [/] Write transaction test suite — install, remove, upgrade, conflict and rollback tests (#58)
  - [x] End-to-end install extraction test
  - [x] End-to-end remove transaction test
  - [ ] Upgrade end-to-end test
  - [ ] Conflict resolution and rollback tests

## Phase 8 · Full Migration to Native Rust <!-- phase:phase-8:migration -->

> **Skipped** — Phase 2 was bypassed, so there is nothing to migrate.

- [x] Replace alpm.rs FFI bindings with native Rust implementation (#36)
- [x] Remove libalpm C dependency (#37)
- [ ] Run comparative benchmarks vs pacman (#38)
  - [ ] Benchmark sync, install and resolve performance
  - [ ] Stress test with full Arch repository — ensure correctness at scale
- [ ] Complete test suite — unit, integration, and fuzzing (#39)
  - [ ] Audit error handling and edge cases — partial downloads, corrupt packages, disk full

## Phase 9 · Future Goals — Post v1.0 <!-- phase:phase-9:future -->

- [ ] Implement Python bindings (#40)
- [ ] Implement internationalization — i18n (#41)
- [ ] Integrate emerging cryptographic standards (#42)
- [ ] Optional TUI interface with ratatui (#43)
- [ ] Smart mirror selection (#44)
- [ ] Configurable package cache (#45)
- [ ] Implement translations — multi-language support based on system locale (#55)

## Phase 10 · Security Execution Track <!-- phase:phase-10:security-execution -->

- [x] Add detached signature verification for sync databases and packages (#61)
  - [x] Download and validate `.db.sig` during `xpm sync`
  - [x] Download and validate package `.sig` during `xpm install`
  - [x] Enforce required/optional/never behavior from `sig_level`
- [x] Add keyring loading and trust source selection (#62)
  - [x] Load trusted OpenPGP certs from configured keyring path(s)
  - [x] Surface clear errors when keyring is missing in `required` mode
- [x] Add security-focused tests for signature enforcement (#63)
  - [x] Sync fails on invalid signature when `sig_level = required`
  - [x] Install fails on invalid signature when `sig_level = required`
  - [x] Optional mode allows unsigned artifacts with warnings

---

## Phase Diagram

```mermaid
gantt
    title xpm Roadmap
    dateFormat  YYYY-MM
    axisFormat  %b %Y

    section Foundation
    Scaffolding           :done, f0, 2026-03, 2w
    CLI and configuration :done, f1, after f0, 3w

    section Transitional Bridge
    libalpm bindings (skipped) :done, f2, after f1, 0d

    section Native Engine
    Dependency resolution :done, f3, after f2, 5w
    Package format        :f4, after f3, 4w
    Repository database   :f5, after f4, 5w

    section Security
    Verification and signing :f6, after f5, 4w

    section System
    Transactions          :f7, after f6, 5w
`
    section Hardening
    Benchmarks and testing :f8, after f7, 4w

    section Post v1.0
    Future goals          :f9, after f8, 8w
```

---

> **Versioning convention:**
> - `v0.1.0` — Phases 0-1 complete (functional CLI with configuration)
> - `v0.2.0` — Phase 2 skipped (no libalpm dependency)
> - `v0.5.0` — Phases 3-5 complete (native engine operational)
> - `v0.8.0` — Phases 6-7 complete (security + transactions)
> - `v1.0.0` — Phase 8 complete (benchmarked, tested, production-ready)

Nota: ya no hay sincronizacion roadmap->issues (workflow eliminado).
