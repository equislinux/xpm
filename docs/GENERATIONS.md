# xpm — Generation integration plan

Context: X Linux versions the system with **generations** (immutable btrfs
snapshot + manifest per change; `xlnux/scripts`, `docs/en/generations.md`).
`xpm` is today postponed (see `docs/en/future-integration.md`) but is the
native package manager of the distribution. This document proposes the
seams so that, when xpm is re-activated, its transactions are
generation-aware **without coupling xpm to x-scripts**.

## Goals

- A successful xpm transaction ends in a new generation (when generations are
  available); a failed one leaves recoverable evidence.
- No dependency from xpm to the provisioning payload: journal + hooks are the
  whole contract.
- Machine-readable history, so a generation manifest can record xpm-managed
  packages without parsing human output.

## Proposal

### 1. Transaction journal — **implemented** (`xpm-core::journal`)

Write `<db_path>/journal/<epoch>-<pid>.json` (default
`/var/lib/xpm/journal/`), written **before** touching files and finalized
after (`xpm history` reads it back):

```json
{
  "schema": 1,
  "action": "install|remove|upgrade",
  "root_dir": "/",
  "started": "2026-09-30T12:00:00Z",
  "finished": "2026-09-30T12:00:04Z",
  "result": "ok|failed",
  "packages": [
    {"name": "kitty", "from": null, "to": "0.44.0-1", "repo": "x", "sha256": "...", "source": "..."}
  ],
  "scriptlets": ["post_install"],
  "error": null
}
```

Timestamps are epoch seconds in the JSON (`started`/`finished`); `xpm history`
renders them as ISO-8601 (`summary()`). `sha256`/`source` are the next addition
(they come from the repo metadata xpm already parses: `SHA256SUM`, `URL`
extended fields, see `docs/INTEGRATION.md`). The transaction engine already has
a `rollback()` primitive and a state machine
(`crates/xpm-core/src/transaction.rs`); the journal is the persistent record of
it. `install`, `remove` and `upgrade` already write it via
`commit_transaction`, and finish it as `ok`/`failed`.

### 2. Hook directories (contract) — **implemented** (`xpm-core::txhooks`)

`/usr/lib/xpm/hooks/pre-transaction.d/*` and `post-transaction.d/*` (override
with `XPM_HOOKS_DIR`), executed in lexical order with the environment:

| Variable | Meaning |
|----------|---------|
| `XPM_ROOT_DIR` | Target root |
| `XPM_ACTION` | `install`, `remove`, `upgrade` |
| `XPM_JOURNAL` | Path of the transaction journal |
| `XPM_PKG_NAMES` / `XPM_PKG_VERSIONS` | Space-separated lists |

Failure policy: a **pre** hook failure aborts the transaction (no changes); a
**post** hook failure logs a warning and never fails the transaction.

`x-scripts` would ship `10-x-gen-pre` (safety generation) and `10-x-gen-post`
(`x gen new --reason xpm:<action>`) only when generations are supported.
xpm stays unaware of snapshots, mirrors or `/var/lib/x`. The runner is
already wired around prepare/commit; only the hook scripts are pending.

### 3. History and rollback

- `xpm history [--json]` — **implemented**: reads the journal, newest first
  (human summary or one JSON object per line). Linking generation ids is
  pending (needs the hooks above).
- `xpm rollback --last` — prints (or executes) the recovery path. Full system
  recovery remains `x gen rollback` (F1 of the design): snapshot ownership
  stays in the provisioning payload, not in xpm.
- `xpm diff <generation>` — later: transaction packages vs the generation
  manifest's `packages.tsv`.

### 4. Stable machine output — **implemented**

`xpm query --format tsv` prints `name<TAB>version` (plain by default);
filtering and `--upgrades` work. Install-reason metadata is tracked as
`<db_path>/local/<pkg>/reason` (`explicit` or `dep`): `xpm install` writes it
(default `explicit`, `--as-deps` for dependencies; using both flags is an
error) and `xpm upgrade` preserves the previous value. A package without a
`reason` file (installed before this feature) counts as `explicit`.
`--explicit`/`--deps` filter on it.

Each installed package also carries the metadata the generation restore path
needs:

- `<db_path>/local/<pkg>/files` — pacman-compatible manifest (`%FILES%` header,
  relative paths, directories with a trailing `/`) derived from the package's
  `.MTREE`, so it covers files, directories and symlinks. This is exactly what
  `xgen_restore_pkg` (`scripts/install/helpers/xgen.sh`) consumes: it skips
  `%` lines and restores the rest. `xpm install`/`xpm upgrade` write it, and
  `xpm files <pkg>` reads it back.
- `<db_path>/local/<pkg>/origin` — name of the repository the package came
  from; absent for a local-file install. `xpm info <pkg>` shows it (plus the
  sync entry's repository, description and dependencies when the sync database
  is present, with the same repository priority as `read_latest_remote_entries`).
- `<db_path>/local/<pkg>/depends` — declared runtime dependencies (raw specs
  such as `libc>=2.39`), recorded by `xpm install`/`xpm upgrade` from the
  package's `.PKGINFO`.
- `<db_path>/local/<pkg>/provides` — virtual names the package provides, used
  to resolve which installed package satisfies a dependency.

Missing `reason`, `origin` or `files` files (legacy or local-file installs)
never fail: they default to `explicit`, `None` and an empty list. `depends`
reads as `None` when absent and `provides` as empty.

`--orphans` walks the recorded dependency edges: a `reason: dep` package is an
orphan when it is not reachable from any explicitly installed package (directly
or transitively, version constraints stripped, `provides` taken into account).
Legacy entries without a `depends` record are never reported, because their
edges are unknown.

### 5. Version pinning and downgrade

`xpm install <pkg>=<ver>` (and upgrade targets) need old versions to exist in
the repository; that is the xpkg side (`xpkg/docs/GENERATIONS.md`: retention +
`history.json`). Rule of thumb to document: partial downgrades only for leaf
packages; kernel/glibc-class changes are handled with a full generation
rollback.

## Preconditions (mapped to the roadmap)

| Item | Where it stands |
|------|-----------------|
| Resolver wired into the CLI | Done for `install` (`resolver::resolve_closure`): dependency closure and order, `name=version`, unversioned `provides`; `upgrade` still compares versions |
| Stub commands (`query`, `files`, ...) | `query` (including `--orphans`), `files`, `info` and `search` implemented |
| Transaction hardening (`.pacnew`, hooks, rollback tests) | Open items in `ROADMAP.md` Phase 7/8 |
| Config keyring path inconsistency | Pending reconciliation |

The **journal + hooks** slice lands on top of the resolver: `install` resolves
the closure before journaling, and the journal records what was done.

## Non-goals

- xpm does not create snapshots, boot entries or `/var/lib/x/generations`.
- No hard dependency on `x-scripts`; if the hook files are absent, xpm works
  exactly as today.

## Phases

1. ~~Transaction journal + `xpm history --json`.~~ done.
2. ~~Hook directories + environment contract.~~ runner done (hook scripts in
   x-scripts pending; `XPM_*` env covered by unit tests).
3. ~~`xpm query --format tsv`.~~ done.
4. `xpm rollback --last` (guidance) + `xpm diff`; `history` links generation
   ids; ~~install-reason metadata~~ done (section 4: `reason` file,
   `--explicit/--deps`); ~~`origin` provenance + package `files` manifest~~ done
   (section 4: `local/<pkg>/origin` and `%FILES%` manifest from `.MTREE`, plus
   `xpm files`/`xpm info`); ~~`--orphans`~~ done (dependency edges recorded at
   install).
5. Version pinning consumed from the xpkg history index.

See also: `../scripts/docs/en/generations.md` (engine),
`../xpkg/docs/GENERATIONS.md` (version retention).
