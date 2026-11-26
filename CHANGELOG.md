# Changelog

All notable changes to this project are documented in this file.

## [Unreleased] - 2026-11-26

### Changed

- Split the `xpm` binary crate into focused modules: `app/` for process setup
  (logging and configuration loading) and `commands/` with one module per
  command family. `main.rs` now only wires the process together.
- Split `commands/install` and `commands/upgrade` into explicit `plan` and
  `execute` phases, preserving the existing download, verification, hook,
  journal and transaction order.
- Split `pacman_compat` into `flags`, `rewrite` and per-operation builders under
  `pacman_compat/build/`.
- Split the CLI definitions into `cli/args/` modules grouped by command family.
- Split `xpm-core` into focused module folders: `hooks/`, `transaction/`,
  `alpm_hooks/`, `pacman_conf/`, `depcheck/`, `conflict/`, `resolver/plan/`,
  `repo_sync/`, `repo_db/`, `local_db/`, `config/`, `check/`, `query/`,
  `journal/`, `replace/` and `package/reader/`.
- Split `xpm-makepkg` chroot handling into `chroot/{template,clone,prepare,mounts,run,cleanup,util}`.
- Split `xpm-progress` into `render`, `settings`, `parallel`, `operation` and
  `phase` modules.
- Moved inline unit tests into `tests.rs`/`tests/` next to the module they
  cover. No test was removed: the suite reports the same 479 tests before and
  after the refactor.
- Grouped large integration test suites under `tests/<suite>/main.rs` with
  feature modules (`cli_journey`, `cli_pacman_compat_flags`, `cli_query_repair`)
  while keeping the same Cargo test target names.
- Centralized shared helpers: `shell_quote` in `xpm-core/src/shell.rs`,
  interactive confirmations in `commands/common.rs`, and configuration
  discovery/overrides in `app/config.rs`.
- Kept public module paths stable through `pub use` re-exports from each module
  root (`mod.rs`).

### Added

- `docs/en/module-map.md`: per-file source index, a "where do I find…?" table,
  testing conventions and a list of known duplication left for a dedicated
  change.

### Documentation

- Updated `docs/en/architecture.md`, `docs/en/usage.md`,
  `docs/en/overview-and-status.md` and `docs/GENERATIONS.md` to match the new
  layout.
- Documented deferred duplication (archive/compression openers, `sha256_file`,
  local-database readers and the shared install/upgrade pipeline) instead of
  merging it in this refactor.

### Validation

- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --workspace` (479 tests) and `scripts/ci-local.sh` pass.
- No functional changes are intended; this branch is a structural
  modularization only.
