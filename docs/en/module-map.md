# xpm — Module Map

This document is the fast index for the `xpm` source tree. Use it to find the
module that owns a behavior before opening a file. The layout follows three
rules:

1. **One responsibility per module.** Files that grew beyond a few hundred
   lines were split by feature, not by line count.
2. **Tests live next to the code they test.** A `tests.rs` (or `tests/` folder)
   inside a module is always the unit-test body of that module; production
   files contain no inline test blocks.
3. **Small shared helpers have one home.** Shell quoting, confirmation prompts,
   installed-package readers and transaction helpers are centralized instead of
   duplicated across commands.

For the higher-level package-manager architecture, see
[`architecture.md`](architecture.md).

---

## Workspace

| Crate | Path | Role |
|-------|------|------|
| `xpm` | `crates/xpm` | CLI frontend: argument parsing, config loading, command handlers |
| `xpm-core` | `crates/xpm-core` | Package-manager engine: ALPM formats, resolver, hooks, transactions, repositories |
| `xpm-makepkg` | `crates/xpm-makepkg` | Source builds: `makepkg` invocation and clean chroot lifecycle |
| `xpm-progress` | `crates/xpm-progress` | Terminal progress rendering shared by the other crates |

---

## `crates/xpm` — CLI frontend

```
src/
├── main.rs                     Process entry: SIGPIPE, clap, logging, dispatch
├── cli/                        clap definitions
│   ├── mod.rs                  Cli, Command, aliases
│   └── args/                   Per-command argument structs
├── app/
│   ├── config.rs               Config discovery, pacman.conf bridging, CLI overrides
│   └── logging.rs              tracing setup from -v flags / RUST_LOG
├── commands/
│   ├── mod.rs                  Module list, shared imports and dispatch()
│   ├── common.rs               confirm_action/confirm_replace, path display, exit_with
│   ├── installed.rs            Installed/sync readers and transaction manifests
│   ├── transaction.rs          Hook/journal/commit orchestration and crash recovery entry
│   ├── sync.rs                 Database sync (+ parallel sync workers)
│   ├── bootstrap.rs            Bootstrap an empty root (and its tests)
│   ├── build.rs                `xpm build` via xpm-makepkg
│   ├── install/
│   │   ├── mod.rs              `xpm install` entry + InstallPlan
│   │   ├── plan.rs             Local files, dependency resolution, --print
│   │   ├── execute.rs          Prefetch, verify, transact and record
│   │   └── download.rs         Parallel prefetch and signature verification
│   ├── remove.rs               `xpm remove`
│   ├── upgrade/
│   │   ├── mod.rs              `xpm upgrade` entry
│   │   ├── plan.rs             Replacement/upgrade plan and reporting
│   │   └── execute.rs          Download, verify, transact and record
│   ├── query.rs                query / search / info / files
│   ├── file_db.rs              `xpm file-db`
│   ├── check.rs                `xpm check` (-Qk/-Qkk)
│   ├── history.rs              history / diff / rollback
│   ├── repo.rs                 repo / groups / list / clean / import-pacman
│   ├── misc.rs                 mark / deptest
│   └── help/
│       ├── mod.rs              Help entry point
│       ├── topics.rs           overview, command list, config, repositories
│       └── command/            Per-command long help
│           ├── mod.rs          Dispatcher
│           ├── sync.rs         sync/install/remove/upgrade
│           ├── query.rs        query/check/mark/deptest
│           ├── inspect.rs      search/info/files
│           └── repo.rs         repo/history/rollback/diff
└── pacman_compat/
    ├── mod.rs                  argv entry, native-token detection, try_rewrite
    ├── flags.rs                Flags/Bundle models and long-option parsing
    ├── rewrite.rs              try_rewrite orchestration
    ├── build/                  pacman operation -> xpm command builders
    │   ├── mod.rs              build_command dispatch
    │   ├── sync.rs             `-S`
    │   ├── query.rs            `-Q`
    │   ├── remove.rs           `-R`
    │   ├── install.rs          `-U`
    │   ├── files_db.rs         `-F`
    │   ├── mark.rs             `-D`
    │   └── deptest.rs          `-T`
    └── tests.rs                argv translation tests
```

Key boundaries:

- `main.rs` only wires process setup and calls `app::*` + `commands::dispatch`.
- A command module owns user interaction and orchestration; reusable database
  and transaction logic lives in `commands/installed.rs` and
  `commands/transaction.rs`.
- `pacman_compat` is pure argv translation. It never touches the filesystem.

## `crates/xpm-core` — engine

```
src/
├── lib.rs                      Public module list and re-exports
├── shell.rs                    Shell quoting shared by scriptlets/hooks
├── config/                     TOML configuration
│   ├── mod.rs                  Defaults and public re-exports
│   ├── types.rs                SigLevel / Repository / GeneralOptions / XpmConfig
│   ├── load.rs                 Loading, validation and CLI overrides
│   ├── pacman.rs               pacman.conf -> XpmConfig lowering
│   └── tests.rs                Config tests
├── error.rs                    XpmError / ConfigError
├── check/                      Installed-file verification (`-Qk` / `-Qkk`)
│   ├── mod.rs                  Shared normalization and re-exports
│   ├── package.rs              PackageCheck and check_package
│   ├── manifest.rs             Fast manifest checks and manifest repair
│   ├── mtree.rs                Full .MTREE verification
│   └── tests.rs                Check tests
├── local_db/                   Per-package local database files
│   ├── mod.rs                  Constants, list IO, re-exports
│   ├── mtree.rs                .MTREE path/hash conversion
│   ├── backup.rs               Backup lists and stored .MTREE
│   ├── files.rs                `files` manifest IO
│   ├── meta.rs                 Origin, version, changelog, deps/provides/groups
│   ├── record.rs               Atomic whole-package metadata records
│   └── tests.rs                Local DB tests
├── repo_db/                    Sync/.files database parsing
│   ├── mod.rs                  Public re-exports
│   ├── types.rs                RepoEntry / SyncDb / LocalDb
│   ├── parse.rs                ALPM archive and section parsing
│   ├── sync_db.rs              .db parsing and JSON cache
│   ├── files_db.rs             .files merging
│   ├── query.rs                Search matching
│   └── tests.rs                Repo DB tests
├── package/                    .xp / .pkg.tar.zst readers and metadata
│   ├── mod.rs, pkginfo.rs, buildinfo.rs, mtree.rs,
│   ├── types.rs, validate.rs,
│   └── reader/                 Package archive reader
│       ├── mod.rs              Re-exports
│       ├── compression.rs      Magic-byte detection and archive opening
│       ├── raw.rs              Raw metadata entries
│       ├── metadata.rs         PackageMetadata / read_metadata
│       ├── files.rs            File listings
│       ├── extract.rs          Payload extraction
│       └── tests.rs            Reader tests
├── hooks/                      Transaction hooks (filesystem lifecycle)
│   ├── mod.rs                  Hook trait, HookContext, ScriptletMode, HookChain
│   ├── extraction/             File extraction and config-file handling
│   │   ├── mod.rs              FileExtractionHook
│   │   ├── backup.rs           .pacnew/.pacsave and stale-file cleanup
│   │   └── mtree.rs            Installed/packaged MTREE hash maps
│   ├── fsutil.rs               Archive opening, entry attrs, path/hash helpers
│   ├── scriptlet.rs            Pre/PostScriptletHook, shell shims/integration
│   ├── local_db.rs             LocalDbHook
│   ├── removal.rs              FileRemovalHook, empty-directory pruning
│   ├── metadata.rs             MetadataLoadHook
│   └── tests.rs                Hook unit tests
├── transaction/                Plan -> prepare -> commit engine
│   ├── mod.rs                  Transaction, constructors, setters, Drop
│   ├── lock.rs                 FileLock
│   ├── op.rs                   TransactionOp / TransactionState
│   ├── plan.rs                 add_install/add_upgrade/add_remove
│   ├── prepare.rs              Pre-flight validation and permissions
│   ├── commit.rs               Commit, parallel batch, version writing
│   ├── execute.rs              Per-operation filesystem execution
│   ├── fast_install.rs         Parallel pure-install extraction
│   ├── accessors.rs            Logging and state accessors
│   └── tests.rs                Transaction unit/e2e tests
├── alpm_hooks/                 pacman `.hook` compatibility
│   ├── mod.rs                  Constants, re-exports
│   ├── types.rs                HookWhen/Trigger/AlpmHook and trigger matching
│   ├── parse.rs                .hook parsing and directory discovery
│   ├── exec.rs                 Execution, chrooting, pacman query shim
│   ├── query.rs                Glob matching and installed-name sets
│   └── tests.rs                Hook unit tests
├── pacman_conf/                pacman.conf reader
│   ├── mod.rs                  Shared errors/values, re-exports
│   ├── types.rs                Usage, SigLevel*, PacmanRepo/Options/Config
│   ├── builders.rs             Repo/Options builders
│   ├── parser/                 Line-oriented parser
│   │   ├── mod.rs              Entry point, file loop and sections
│   │   ├── directives.rs       [options] directive handlers
│   │   └── repo.rs             Repository and Include handlers
│   ├── include.rs              Glob include expansion
│   ├── siglevel.rs             SigLevel/Usage/ParallelDownloads value parsers
│   └── tests.rs                Config-parser tests
├── resolver/                   SAT-based dependency resolution
│   ├── mod.rs                  Re-exports, assume-installed candidates
│   ├── version.rs              ALPM vercmp
│   ├── dependency.rs           DepConstraint parsing/matching
│   ├── types.rs                PackageCandidate / PackagePool
│   ├── provider.rs             resolvo DependencyProvider bridge
│   ├── plan/                   High-level plan construction
│   │   ├── mod.rs              Requirement and re-exports
│   │   ├── closure.rs          Closure resolution and dependency ordering
│   │   ├── conflicts.rs        Solution/installed conflict checks
│   │   ├── sat.rs              Pool interning helpers
│   │   └── tests.rs            Resolver tests
│   └── tests.rs                Cross-module resolver tests
├── depcheck/                   Removal and install-time checks
│   ├── mod.rs                  InstalledPkg / RemoveOptions, re-exports
│   ├── removal/                -R planning and ordering
│   │   ├── mod.rs              Plan entry, set helpers, satisfies
│   │   ├── recursive.rs        Reverse-dependency discovery/selection
│   │   └── order.rs            Dependency ordering
│   ├── install.rs              InstallTarget, check_install, closure checks
│   ├── install_checks.rs       Install-check tests
│   └── tests.rs                Removal tests
├── conflict/                   File-conflict detection
│   ├── mod.rs                  Public types and checks
│   ├── types.rs                InstalledManifest/TargetManifest/ConflictKind
│   ├── overwrite.rs            --overwrite pattern parsing/matching
│   ├── normalize.rs            Path normalization/sorting
│   ├── owner.rs                Ownership index and directory ownership
│   ├── check.rs                Target/filesystem conflict scans
│   └── tests.rs                Conflict tests
├── repo_sync/                  Remote repository operations
│   ├── mod.rs                  RepoSyncResult, expand_repo_url
│   ├── db.rs                   Database/file sync
│   ├── verify.rs               Signatures and SHA-256
│   ├── download.rs             URL candidates and retry loop
│   ├── transfer.rs             HTTP/file transfer, atomic writes
│   ├── progress.rs             Single-line download progress
│   └── tests.rs                Sync/download tests
├── query/                      Pacman-compatible query helpers
│   ├── mod.rs                  Re-exports
│   ├── local.rs                LocalPackage and loading
│   ├── owners.rs               -Qo owner lookup and path normalization
│   ├── filters.rs              -Qm/-Qt filters
│   └── tests.rs                Query tests
├── journal/                    Transaction journal
│   ├── mod.rs                  Schema and re-exports
│   ├── entry.rs                JournalPackage
│   ├── store.rs                Journal records and persistence
│   ├── recovery.rs             Reconciliation of interrupted (`running`) journals
│   ├── time.rs                 Epoch / ISO-8601 helpers
│   └── tests.rs                Journal tests
├── replace/                    Sysupgrade replacement planning
│   ├── mod.rs                  Re-exports
│   ├── patterns.rs             IgnorePkg/IgnoreGroup patterns
│   ├── resolution.rs           Replacement selection
│   └── tests.rs                Replacement tests
├── cache.rs, deptest.rs, generations.rs, groups.rs, install_reason.rs,
├── migrate.rs, orphans.rs, repo.rs, rollback.rs, signing.rs, txhooks.rs
```

## `crates/xpm-makepkg` — clean source builds

```
src/
├── lib.rs                      Build options, artifact discovery, entry point
├── pacman-shim.sh              Query shim used inside the chroot
└── chroot/
    ├── mod.rs                  build_chroot orchestration
    ├── template.rs             Shared base-devel template + lock + stamp
    ├── clone.rs                Snapshot / reflink / overlay / copy
    ├── prepare.rs              Build user, helpers, host keyring
    ├── mounts.rs               Bind mounts and mountpoints
    ├── run.rs                  makepkg invocation and artifact collection
    ├── cleanup.rs              WorkGuard, stale-root cleanup
    ├── util.rs                 root_command, ids, dates, quoting
    └── tests.rs                Chroot tests
```

## `crates/xpm-progress`

```
src/
├── lib.rs                      Public re-exports
├── render.rs                   Percentage, bar layout and line rendering
├── settings.rs                 Process-wide enable/tty/verbose settings
├── parallel.rs                 Aggregate concurrent-download progress
├── operation.rs                Standalone per-operation count bar
├── phase.rs                    Per-phase progress bars
└── tests.rs                    Progress tests
```

It is intentionally dependency-light (`xpm-core` imports it as
`xpm_core::progress`).

---

## Where do I find…?

| Task | Start here |
|------|------------|
| Add or rename a CLI flag | `crates/xpm/src/cli/args/`, then the command module |
| Change how a pacman flag maps to xpm | `crates/xpm/src/pacman_compat/flags.rs` and `build/` |
| Change installation planning/order | `crates/xpm/src/commands/install/`, `commands/installed.rs` |
| Change package extraction, `.pacnew`/`.pacsave` | `crates/xpm-core/src/hooks/extraction.rs`, `fsutil.rs` |
| Change `.INSTALL` scriptlet execution | `crates/xpm-core/src/hooks/scriptlet.rs` |
| Change commit/rollback behavior | `crates/xpm-core/src/transaction/` |
| Change pacman `.hook` compatibility | `crates/xpm-core/src/alpm_hooks/` |
| Change `pacman.conf` parsing | `crates/xpm-core/src/pacman_conf/` |
| Change dependency solving | `crates/xpm-core/src/resolver/plan/` |
| Change configuration loading/validation | `crates/xpm-core/src/config/` |
| Change sync `.db`/`.files` parsing | `crates/xpm-core/src/repo_db/` |
| Change local database files | `crates/xpm-core/src/local_db/` |
| Change installed-file checks | `crates/xpm-core/src/check/` |
| Change owner/foreign/unrequired queries | `crates/xpm-core/src/query/` |
| Change the transaction journal | `crates/xpm-core/src/journal/` |
| Change sysupgrade replacements | `crates/xpm-core/src/replace/` |
| Change removal/install checks | `crates/xpm-core/src/depcheck/` |
| Change file-conflict behavior | `crates/xpm-core/src/conflict/` |
| Change repository sync/download | `crates/xpm-core/src/repo_sync/` |
| Change chroot template or mounts | `crates/xpm-makepkg/src/chroot/` |
| Add a unit test | The `tests.rs` (or `tests/`) next to the module |

## Known duplication (deferred)

The following duplicates are known and intentionally left unchanged; merging
them is a dedicated change with its own tests, not a mechanical split:

- Archive/compression openers exist in `package/reader/compression.rs`,
  `repo_db/parse.rs` and `hooks/fsutil.rs`.
- `sha256_file` exists in `check/mtree.rs`, `hooks/fsutil.rs` and
  `package/validate.rs`.
- Local-database readers in `crates/xpm/src/commands/installed.rs` overlap with
  `xpm-core::local_db` / `xpm-core::query`.
- `install` and `upgrade` share the download → verify → hooks → commit flow;
  see `commands/transaction.rs` before changing either.

## Tests

- Unit tests live next to the module as `tests.rs` (or a `tests/` folder).
- Integration tests live in `crates/xpm/tests/`. Single-file suites stay as
  `*.rs`; grouped suites use `<name>/main.rs` plus feature modules, so the
  test target name and the shared fixture stay stable:
  - `cli_journey/{rollback,hooks,diff_history}.rs`
  - `cli_pacman_compat_flags/{sync_query,files_cache,upgrade}.rs`
  - `cli_query_repair/{query,check,mark}.rs`

## Conventions

- Cross-module helpers are `pub(super)`; the public surface is re-exported from
  the module root (`mod.rs`), so `xpm_core::hooks::FileExtractionHook` and
  `xpm_core::transaction::FileLock` keep working.
- Module roots (`mod.rs`) hold only types, re-exports, shared imports and the
  orchestration function. They should stay small.
- Tests use `use super::*` and remain in the module they test; moving a test
  body to `tests.rs` does not change its module path or visibility.
- Run `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test --workspace` before publishing changes.
