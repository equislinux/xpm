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
├── cli.rs                      clap command/args definitions and aliases
├── app/
│   ├── config.rs               Config discovery, pacman.conf bridging, CLI overrides
│   └── logging.rs              tracing setup from -v flags / RUST_LOG
├── commands/
│   ├── mod.rs                  Module list, shared imports and dispatch()
│   ├── common.rs               confirm_action/confirm_replace, path display, exit_with
│   ├── installed.rs            Installed/sync readers, manifests, InstalledRecord
│   ├── transaction.rs          Hook/journal/commit orchestration shared by commands
│   ├── sync.rs                 Database sync (+ parallel sync workers)
│   ├── bootstrap.rs            Bootstrap an empty root (and its tests)
│   ├── build.rs                `xpm build` via xpm-makepkg
│   ├── install/
│   │   ├── mod.rs              `xpm install` flow
│   │   └── download.rs         Parallel prefetch and signature verification
│   ├── remove.rs               `xpm remove`
│   ├── upgrade.rs              `xpm upgrade` planning and execution
│   ├── query.rs                query / search / info / files
│   ├── file_db.rs              `xpm file-db`
│   ├── check.rs                `xpm check` (-Qk/-Qkk)
│   ├── history.rs              history / diff / rollback
│   ├── repo.rs                 repo / groups / list / clean / import-pacman
│   ├── misc.rs                 mark / deptest
│   └── help/
│       ├── mod.rs              Help entry point
│       ├── topics.rs           overview, command list, config, repositories
│       └── command.rs          Per-command long help
└── pacman_compat/
    ├── mod.rs                  argv entry, native-token detection, try_rewrite
    ├── flags.rs                Flags/Bundle models and long-option parsing
    ├── rewrite.rs              pacman operation -> xpm command builders
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
├── config.rs                   TOML configuration
├── error.rs                    XpmError / ConfigError
├── package/                    .xp / .pkg.tar.zst readers and metadata
│   ├── mod.rs, reader.rs, pkginfo.rs, buildinfo.rs, mtree.rs,
│   ├── types.rs, validate.rs
├── hooks/                      Transaction hooks (filesystem lifecycle)
│   ├── mod.rs                  Hook trait, HookContext, ScriptletMode, HookChain
│   ├── extraction.rs           FileExtractionHook, .pacnew/.pacsave, MTREE handling
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
│   ├── parser.rs               Line parser, Include loop, $arch validation
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
│   ├── removal.rs              -R planning, recursive/cascade, ordering
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
├── cache.rs, check.rs, config.rs, deptest.rs, generations.rs,
├── groups.rs, install_reason.rs, journal.rs, local_db.rs, migrate.rs,
├── orphans.rs, query.rs, replace.rs, repo.rs, repo_db.rs, rollback.rs,
├── signing.rs, txhooks.rs
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

`src/lib.rs` renders phase bars, per-package progress and parallel download
aggregates. It is intentionally dependency-light (`xpm-core` imports it as
`xpm_core::progress`).

---

## Where do I find…?

| Task | Start here |
|------|------------|
| Add or rename a CLI flag | `crates/xpm/src/cli.rs`, then the command module |
| Change how a pacman flag maps to xpm | `crates/xpm/src/pacman_compat/flags.rs` and `rewrite.rs` |
| Change installation planning/order | `crates/xpm/src/commands/install/`, `commands/installed.rs` |
| Change package extraction, `.pacnew`/`.pacsave` | `crates/xpm-core/src/hooks/extraction.rs`, `fsutil.rs` |
| Change `.INSTALL` scriptlet execution | `crates/xpm-core/src/hooks/scriptlet.rs` |
| Change commit/rollback behavior | `crates/xpm-core/src/transaction/` |
| Change pacman `.hook` compatibility | `crates/xpm-core/src/alpm_hooks/` |
| Change `pacman.conf` parsing | `crates/xpm-core/src/pacman_conf/` |
| Change dependency solving | `crates/xpm-core/src/resolver/plan/` |
| Change removal/install checks | `crates/xpm-core/src/depcheck/` |
| Change file-conflict behavior | `crates/xpm-core/src/conflict/` |
| Change repository sync/download | `crates/xpm-core/src/repo_sync/` |
| Change chroot template or mounts | `crates/xpm-makepkg/src/chroot/` |
| Add a unit test | The `tests.rs` (or `tests/`) next to the module |

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
