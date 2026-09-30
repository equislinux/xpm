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

### 1. Transaction journal (xpm side)

Write `/var/lib/xpm/journal/<epoch>-<pid>.json`, fsynced **before** touching
files and finalized after:

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

`sha256`/`source` come from the repo metadata xpm already parses (`SHA256SUM`,
`URL` extended fields, see `docs/INTEGRATION.md`). The transaction engine
already has a `rollback()` primitive and a state machine
(`crates/xpm-core/src/transaction.rs`); the journal is the persistent record of
it.

### 2. Hook directories (contract)

`/usr/lib/xpm/hooks/pre-transaction.d/*` and `post-transaction.d/*`, executed
in lexical order with the environment:

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
xpm stays unaware of snapshots, mirrors or `/var/lib/x`.

### 3. History and rollback

- `xpm history [--json]` — reads the journal; if generations exist, appends the
  generation id created after each transaction.
- `xpm rollback --last` — prints (or executes) the recovery path. Full system
  recovery remains `x gen rollback` (F1 of the design): snapshot ownership
  stays in the provisioning payload, not in xpm.
- `xpm diff <generation>` — later: transaction packages vs the generation
  manifest's `packages.tsv`.

### 4. Stable machine output

`xpm query --format tsv` (or `xpm query --manifest`): `name`, `version`,
`origin`, `explicit|dep`. Today `xgen_capture_packages` falls back to
`xpm query` human output; this makes the capture exact.

### 5. Version pinning and downgrade

`xpm install <pkg>=<ver>` (and upgrade targets) need old versions to exist in
the repository; that is the xpkg side (`xpkg/docs/GENERATIONS.md`: retention +
`history.json`). Rule of thumb to document: partial downgrades only for leaf
packages; kernel/glibc-class changes are handled with a full generation
rollback.

## Preconditions (mapped to the roadmap)

| Item | Where it stands |
|------|-----------------|
| Resolver wired into the CLI | Pending (`future-integration.md` item 1); needed before upgrade journaling is meaningful |
| Stub commands (`query`, `files`, ...) | Partially pending; journal/hooks do not need them |
| Transaction hardening (`.pacnew`, hooks, rollback tests) | Open items in `ROADMAP.md` Phase 7/8 |
| Config keyring path inconsistency | Pending reconciliation |

The **journal + hooks** slice can be implemented before the resolver: the
install path already selects packages by name, and the journal only records
what was done.

## Non-goals

- xpm does not create snapshots, boot entries or `/var/lib/x/generations`.
- No hard dependency on `x-scripts`; if the hook files are absent, xpm works
  exactly as today.

## Suggested phases

1. Transaction journal + `xpm history --json`.
2. Hook directories + environment contract.
3. `xpm query --format tsv`.
4. `xpm rollback --last` (guidance) + `xpm diff`; `history` links generation ids.
5. Version pinning consumed from the xpkg history index.

See also: `../scripts/docs/en/generations.md` (engine),
`../xpkg/docs/GENERATIONS.md` (version retention).
