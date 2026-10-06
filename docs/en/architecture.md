# xpm — Architecture

Technical architecture of `xpm`, based on the workspace layout, the crate source under
`crates/`, the README, and the docs in this folder.

---

## Workspace and crates

The repository is a Cargo workspace with two members:

```
xpm/
├── Cargo.toml                  # workspace root (resolver 2, shared deps, version 0.1.0)
├── crates/
│   ├── xpm/                    # binary crate — CLI frontend
│   │   └── src/
│   │       ├── main.rs         # entry point, logging, config load, subcommand dispatch
│   │       └── cli.rs          # clap CLI definitions (commands, aliases, flags)
│   └── xpm-core/               # library crate — core logic
│       └── src/
│           ├── lib.rs          # module root and re-exports
│           ├── config.rs       # TOML configuration parser
│           ├── error.rs        # ConfigError / XpmError types
│           ├── repo.rs         # repository manager (user repos in /etc/xpm.d/)
│           ├── repo_db.rs      # ALPM-style .db / .files parsing
│           ├── repo_sync.rs    # remote database download / sync
│           ├── signing.rs      # OpenPGP detached signature verification
│           ├── hooks.rs        # Hook trait + built-in transaction hooks
│           ├── transaction.rs  # transaction engine (plan/prepare/commit)
│           ├── package/        # .xp / .pkg.tar.zst readers and parsers
│           │   ├── reader.rs, pkginfo.rs, buildinfo.rs, mtree.rs,
│           │   ├── types.rs, validate.rs
│           └── resolver/       # SAT-based dependency resolution
│               ├── version.rs, dependency.rs, types.rs, provider.rs
├── etc/xpm.conf.example        # example configuration
├── tests/                      # repo database integration tests
├── ROADMAP.md                  # internal development roadmap
└── docs/                       # documentation (this folder)
```

The binary crate (`xpm`) depends only on `clap`, `anyhow`, `tracing`, `tracing-subscriber`, and
`xpm-core`. All domain logic lives in `xpm-core`.

## CLI dispatch flow

`main.rs` follows a straight-line pipeline:

1. Parse the CLI with clap (`Cli::parse`).
2. Initialize `tracing` logging; verbosity `-v`/`-vv`/`-vvv` maps to INFO/DEBUG/TRACE
   (default WARN). `RUST_LOG` overrides via `EnvFilter`.
3. Load the configuration from `--config` (default `/etc/xpm.conf`); if the file is missing the
   parser falls back to built-in defaults (`load_or_default`).
4. Apply CLI overrides for `--root`, `--dbpath`, `--cachedir`, and `--no-color`.
5. Dispatch to the handler of the parsed subcommand.

## Dependency resolution

The resolver lives in `crates/xpm-core/src/resolver/` and delegates all SAT mechanics to the
`resolvo` crate (CDCL, watched literals, unit propagation). xpm supplies the data model and the
`DependencyProvider` implementation. Four modules:

| Module | File | Purpose |
|--------|------|---------|
| `version` | `version.rs` | ALPM `vercmp`-compatible version parsing/comparison |
| `dependency` | `dependency.rs` | Dependency constraint parsing and matching |
| `types` | `types.rs` | Package pool and interning bridge to resolvo |
| `provider` | `provider.rs` | `XpmProvider` implementing resolvo `Interner` + `DependencyProvider` |

Key types:

- `Version` — parsed ALPM version `[epoch:]pkgver[-pkgrel]`; segment-by-segment comparison
  (numeric vs. alphabetic) matching `vercmp`.
- `Operator` — `Ge (>=)`, `Le (<=)`, `Gt (>)`, `Lt (<)`, `Eq (=)`.
- `DepConstraint` — a parsed dependency such as `glibc>=2.38` (`name`, optional `op`, optional
  `version`); `matches(&Version)` evaluates a candidate.
- `PackageCandidate` — one installable version with `name`, `version`, `depends`, `conflicts`,
  `provides`, `optdepends`.
- `PackagePool` — interning arena mapping names, candidates, version sets, version set unions,
  and strings to resolvo's opaque IDs. Conflicts use `intern_conflict_version_set`, which stores
  the constraint with `negated = true` so resolvo forbids candidates that *do* match.
- `XpmProvider` — feeds the pool into the solver (`filter_candidates`, `get_candidates`,
  `sort_candidates` highest-first, `get_dependencies` mapping `depends` to requirements and
  `conflicts` to constrains).

The README expresses the model as CNF clauses:

| Requirement | CNF clause | Meaning |
|-------------|-----------|---------|
| Dependency | `!foo OR bar` | If `foo` is installed, `bar` must be too |
| Root requirement | `foo` | Target package is mandatory |
| Conflict | `!bar_v1 OR !bar_v2` | Mutually exclusive versions |

Honest note: the resolver is implemented and tested at the library level (see
`docs/RESOLVER.md`), but the current CLI `install`/`upgrade` handlers in `main.rs` do not invoke
it — `install` selects a package by exact name from the synced database, and `upgrade` compares
versions with `Version::cmp_versions`. Using the solver from the CLI is future work.

## Package format

xpm reads `.xp` packages natively and stays compatible with Arch `.pkg.tar.zst`. Both are
`tar.zst` archives (zstd compression; the reader also auto-detects gzip and xz from magic bytes)
containing metadata entries:

| Entry | Content |
|-------|---------|
| `.PKGINFO` | `key = value` metadata: name, version+release, description, URL, build date, size, arch, licenses, `depends`, `makedepends`, `checkdepends`, `optdepends`, `provides`, `conflicts`, `replaces` |
| `.BUILDINFO` | Reproducible build environment record |
| `.MTREE` | File manifest with modes, sizes, and SHA-256 digests (`sha256digest=`) |
| `.INSTALL` | Optional bash scriptlets (`pre_install`, `post_install`, `pre_upgrade`, `post_upgrade`, `pre_remove`, `post_remove`) |

`crates/xpm-core/src/package/reader.rs` provides `read_metadata`, `list_files`,
`extract_to`, and `read_raw_entry`; the parsers live in `pkginfo.rs`, `buildinfo.rs`, and
`mtree.rs`; shared structs in `types.rs`. `validate.rs` implements post-extraction integrity
validation: extracted files are checked against the `.MTREE` manifest for SHA-256 checksums,
sizes, and file types.

## Repository database and sync

`repo_db.rs` parses ALPM-style repository databases:

- `RepoEntry` — one package with metadata plus optional xpkg extensions: `filename`,
  `sha256sum`, `url`, and a `files` list.
- `SyncDb` / `LocalDb` — parsed sync database and the local installed database.
- `parse_sync_db`, `merge_files_db` — parse the `.db` archive and merge the optional `.files`
  listing.

`repo_sync.rs` handles remote operations:

- `expand_repo_url` — substitutes `$repo` and `$arch` in mirror URLs.
- `sync_repo_databases` — downloads `<repo>.db` (mandatory) and `<repo>.files` (best effort)
  into the sync directory, trying mirrors in order with retries.
- `verify_remote_signature` / `verify_sha256` — signature and checksum enforcement.
- `package_download_candidates` / `download_first_available` — build candidate URLs for a
  package and download from the first reachable mirror.

Sync databases land under `<db_path>/sync/` (e.g. `/var/lib/xpm/sync/<repo>.db`). The local
database of installed packages lives under `<db_path>/local/<pkg>/` and stores at least a
`version` file and a `files` manifest.

## Signing

`signing.rs` uses `sequoia-openpgp` (pure Rust) to verify detached OpenPGP signatures:

- `load_keyring` — parse a keyring file (e.g. `trustedkeys.gpg`) into a set of certificates.
- `verify_file` / `verify_detached` — verify a detached signature over file bytes.
- `VerifyOutcome` — `Good { key_id }`, `UnknownKey`, or `Bad { reason }`.

Enforcement is controlled by `sig_level` from the config (per-repository override supported):
`required` fails on missing/invalid signatures, `optional` warns, `never` skips verification.

## Transactions and hooks

`transaction.rs` implements the transaction engine with a plan/prepare/commit lifecycle:

- `FileLock` — filesystem lock (under the local DB) preventing concurrent xpm operations.
- `TransactionOp` — `Install { pkg_name, pkg_version, pkg_file }`, `Remove { pkg_name }`,
  `Upgrade { ... }`.
- `TransactionState` — `Planning`, `Prepared`, `Committed`, `RolledBack`.
- `Transaction` — collect operations during planning, run pre-flight preparation, then commit;
  failures trigger rollback logic; operations are appended to the log file (default
  `/var/log/xpm.log` under the root).

`hooks.rs` provides the `Hook` trait and a `HookChain`. The default chain runs, per operation:

1. `MetadataLoadHook` — load package metadata for the operation.
2. `PreScriptletHook` — run `.INSTALL` pre-scriptlets (`pre_install`, `pre_upgrade`,
   `pre_remove`).
3. `FileExtractionHook` — extract package files with correct ownership.
4. `FileRemovalHook` — remove tracked files on uninstall (and prune empty directories outside
   system root).
5. `PostScriptletHook` — run `.INSTALL` post-scriptlets (`post_install`, `post_upgrade`,
   `post_remove`).
6. `LocalDbHook` — register/remove the package in the local database.

Scriptlets are sourced through `bash` with `XPM_ROOT_DIR`, `XPM_PKG_NAME`, and
`XPM_PKG_VERSION` exported.

## Configuration

`/etc/xpm.conf` is a TOML file with `[options]` and one `[[repo]]` block per predefined
repository. Defaults come from `config.rs` and `etc/xpm.conf.example`:

```toml
[options]
root_dir = "/"
db_path = "/var/lib/xpm/"
cache_dir = "/var/cache/xpm/pkg/"
log_file = "/var/log/xpm.log"
gpg_dir = "/etc/pacman.d/gnupg/"     # code default
sig_level = "optional"               # optional | required | never
color = true
parallel_downloads = 5
check_space = true
# architecture = "x86_64"            # auto-detected if unset
# hold_pkg = ["linux"]
# ignore_pkg = [...]
# ignore_group = [...]

[[repo]]
name = "x"
server = ["https://equislinux.github.io/x-repo/x/$arch"]
# sig_level = "required"             # optional per-repo override
```

Notes:

- If the file does not exist, xpm falls back to built-in defaults (`XpmConfig::default`), whose
  single repository is `x` at `https://equislinux.github.io/x-repo/x/$arch`.
- The example file and some README/help snippets differ on the GPG directory default: code uses
  `/etc/pacman.d/gnupg/`, while README guidance uses `/etc/xpm/gnupg/`. This discrepancy is
  worth reconciling before depending on it.
- Config validation rejects `parallel_downloads = 0`, empty repository names, and repositories
  without servers.
- `sig_level` can be set globally or per repository; the per-repo value wins when present.
- User-added repositories are stored individually in `/etc/xpm.d/<name>.toml` and managed with
  `xpm repo add/remove/list`.
- URL variables `$repo` and `$arch` are substituted at fetch time.

## Repository hosting

The default repository is hosted on GitHub Pages at `equislinux.github.io/x-repo` and is reachable as
a static file tree; xpm therefore supports any HTTP-based static file server (VPS migration is
transparent). Predefined mirror order in config decides package priority (first repository that
provides a package wins); within a repository, mirrors are tried in order until one succeeds.

## External dependencies (workspace)

| Area | Crates |
|------|--------|
| CLI | `clap` 4 |
| Serialization | `serde` 1, `toml` 0.8 |
| Errors | `anyhow` 1, `thiserror` 2 |
| Resolution | `resolvo` 0.10, `itertools` 0.14 |
| Archive/compression | `tar` 0.4, `zstd` 0.13, `flate2` 1, `xz2` 0.1 |
| Crypto | `sha2` 0.10, `sequoia-openpgp` 1 (rust crypto, no default C features) |
| Logging | `tracing` 0.1, `tracing-subscriber` 0.3 |
| Network | `reqwest` 0.12 (blocking, rustls-tls) |

## References

- Resolver details and test coverage: [`../RESOLVER.md`](../RESOLVER.md)
- Fetch targets and endpoints: [`../FETCH_TARGETS.md`](../FETCH_TARGETS.md)
- Integration with xpkg: [`../INTEGRATION.md`](../INTEGRATION.md)
- Example config: [`../../etc/xpm.conf.example`](../../etc/xpm.conf.example)
