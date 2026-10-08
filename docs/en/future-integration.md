# xpm — Future Integration Notes

How `xpm` is expected to re-enter the X distribution flow, the preconditions before it can be
the active install path, and the seams that should stay stable in the meantime. This document
records the current understanding from the workspace-level ROADMAP, this repo's README/ROADMAP,
and `docs/INTEGRATION.md`. Nothing here is committed work; it is planning context.

---

## Where xpm stands today

Within the reboot initiative, xpm/xpkg tooling is **postponed** (workspace ROADMAP, Phase 4).
The distro payload is currently packaged with a PKGBUILD + makepkg flow as the `x-scripts`
package and published through the pacman-compatible repository, consumed on installed systems
with pacman. `xpm` is a functioning, well-tested Rust codebase that is not yet wired into the
active reboot path.

The honest consequence: documentation of user-facing behaviour may drift from what a distro
actually ships today, because the distro ships the pacman path, not xpm. Treat xpm features as
the future native path unless the reboot flow adopts them.

## When xpm could become active again

Reasons that would pull xpm (and its companion xpkg) back into scope, per the workspace ROADMAP:

- The distro needs the **native `.xp` repository** as the distribution channel (moving away from
  consuming the pacman layout for x packages).
- The **native install path** is functional (resolver-backed `install` for repo names and local
  `.xp` files, dependency-aware `upgrade`, `.files` database, `rollback`/`diff`, `.pacnew`/
  `.pacsave`); the remaining piece is the native `.xp` repository rollout.
- Reproducible, linted packaging (xpkg) with OpenPGP signatures becomes a hard requirement of
  the payload pipeline.

Until then, xpm should not be treated as a dependency of the distro build, and the reboot flow
should not add assumptions about xpm being installed.

## Preconditions before xpm becomes the active path

Code-level gaps tracked before re-activation. The recovery and hardening items are done
(2026-10-08); only the production-readiness list below remains open:

1. ~~Implement rollback and generation diffs.~~ Done: `rollback --last/--journal` replays the
   inverse from the package cache, `diff <generation>` compares against `packages.tsv`, and
   journals link the generation they produced (`gen:NNNN`).
2. ~~Complete transaction hardening.~~ Done: `.pacnew`/`.pacsave` by `backup` entries,
   pacman-style ALPM hooks around transactions, upgrade end-to-end journey and rollback tests.
3. **Close production-readiness milestones** (repo ROADMAP Phase 8 and Phase 9): scale/stress
   tests against large repositories are in-suite; a direct benchmark vs pacman needs root and
   stays manual. Remaining: fuzzing, deeper error-handling audit (partial downloads, corrupt
   packages, disk full), and post-v1.0 goals (Python bindings, i18n, TUI, smart mirror
   selection, configurable cache).
4. ~~Reconcile config inconsistencies~~ (GPG keyring default): resolved via
   `effective_gpg_dir()` — configured directory, then `/etc/pacman.d/gnupg`, then
   `/etc/xpm/gnupg`.

## Integration with xpkg

`xpm` and `xpkg` are complementary binaries sharing the package format (per `docs/INTEGRATION.md`):

| Tool | Role | Analogy |
|------|------|---------|
| `xpm` | Package manager — install, remove, upgrade, resolve dependencies | `pacman` |
| `xpkg` | Package builder — compile, package, lint, manage repos | `makepkg` + `repo-add` + `namcap` |

Lifecycle: source recipe (XBUILD/PKGBUILD) -> `xpkg build` -> `.xp` package -> repository
database -> static hosting -> `xpm sync` -> `xpm install`/`upgrade` (verify signature, extract,
run `.INSTALL` scriptlets, register locally). xpkg can also emit a plain ALPM-style database;
xpm reads both standard and xpkg-extended fields (`FILENAME`, `SHA256SUM`, `URL`), keeping
backward compatibility with Arch repositories.

The repo's README references `docs/INTEGRATION.md` for how the tools work together; keep that
document in sync whenever the format changes.

## Seams that should stay stable

Because xpm may be dormant for a while, the following interfaces should be treated as
compatibility surfaces and documented carefully when they change:

- **Package format**: `.xp` == ALPM `.pkg.tar.zst` structure (tar.zst with `.PKGINFO`,
  `.BUILDINFO`, `.MTREE`, optional `.INSTALL`). Format version is implicit in the `.PKGINFO`
  structure; both tools should move in lockstep.
- **Repository layout**: sync endpoint `<server>/<repo>.db` (+ `.files`), xpm-native tree at
  `https://equislinux.github.io/x-repo/x/$arch` (do not confuse with the pacman endpoint under
  `/repo/x86_64`), signatures as detached `.sig`, keyring as `trustedkeys.gpg`.
- **Configuration**: `/etc/xpm.conf` (TOML), user repos under `/etc/xpm.d/`, `$repo`/`$arch`
  URL variables, `sig_level` semantics.
- **Local database layout**: `/var/lib/xpm/local/<pkg>/` with `version`, `reason`, `origin`,
  `files`, `depends` and `provides` entries,
  sync DBs under `/var/lib/xpm/sync/`.
- **Scriptlet contract**: bash functions sourced from `.INSTALL` with `XPM_ROOT_DIR`,
  `XPM_PKG_NAME`, `XPM_PKG_VERSION` environment variables.

## Working conventions that also apply later

When xpm work resumes, the workspace conventions apply as they do today: work happens on the
`x/reboot` branch of this repo, commits stay local (no push), commit messages carry no emojis or
external-tool references, and each phase closes with local tests before moving on. Keep any
integration decision recorded in the repo or folder it belongs to.

## See also

- Repo roadmap: `../../ROADMAP.md`
- Workspace-level roadmap (reboot phases, including the postponed tooling phase):
  `../../../ROADMAP.md`
- xpm/xpkg integration: [`../INTEGRATION.md`](../INTEGRATION.md)
- Resolver architecture: [`../RESOLVER.md`](../RESOLVER.md)
