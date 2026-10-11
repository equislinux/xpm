# xpm — Future Integration Notes

How `xpm` fits into the X distribution flow, what remains open before it can be
the main install path, and which interfaces should stay stable.

---

## Where xpm stands today

`xpm` is **wired end to end**:

- The SAT resolver runs inside `install` and `upgrade`: dependency closure,
  dependency order, `name=version` requirements and unversioned `provides`.
- Local `.xp` installs, package removal and dependency-aware upgrades work
  through the same transaction engine.
- The native endpoint (`https://equislinux.github.io/x-repo/x/$arch`, separate
  from the pacman endpoint under `/repo/x86_64`) is published and validated
  with `xpm sync/search/deps/install`.
- The system itself is still provisioned with **pacman + the signed `[x]`
  repository**: `xpm` coexists as the native channel rather than replacing the
  production flow.

## Before xpm becomes the active path

Production-readiness list (the recovery and hardening items closed earlier):

1. **Resolver — `gcc-libs` cycle.** `Unsolvable(Conflict)` for packages that
   depend on `gcc-libs`, even on a clean root.
2. **Hardlink extraction.** Fails on some real Arch packages (`zoneinfo` files
   under `usr/share/zoneinfo/`).
3. **Production milestones** (repo ROADMAP phases 8 and 9): scale/stress tests
   against large repositories are already in-suite; the direct benchmark vs
   pacman needs root and stays manual. Remaining: fuzzing, deeper
   error-handling audit (partial downloads, corrupt packages, disk full) and
   post-v1.0 goals (Python bindings, i18n, TUI, mirror selection, configurable
   cache).

## Integration with xpkg

`xpm` and `xpkg` are complementary binaries sharing the package format:

| Tool | Role | Analogy |
|------|------|---------|
| `xpm` | Package manager — install, remove, upgrade, resolve dependencies | `pacman` |
| `xpkg` | Package builder — compile, package, lint, manage repos | `makepkg` + `repo-add` + `namcap` |

Lifecycle: recipe (XBUILD/PKGBUILD) → `xpkg build` → `.xp` package → repository
database → static hosting → `xpm sync` → `xpm install`/`upgrade` (verify
signature, extract, run `.INSTALL` scriptlets, register locally). xpkg can also
emit a plain ALPM-style database; xpm reads both standard and xpkg-extended
fields (`FILENAME`, `SHA256SUM`, `URL`), keeping backward compatibility with
Arch repositories.

## Interfaces to keep stable

- **Package format**: `.xp` == ALPM `.pkg.tar.zst` structure (tar.zst with
  `.PKGINFO`, `.BUILDINFO`, `.MTREE`, optional `.INSTALL`). The format version
  is implicit in `.PKGINFO`; both tools should move in lockstep.
- **Repository layout**: sync endpoint `<server>/<repo>.db` (+ `.files`),
  native tree at `https://equislinux.github.io/x-repo/x/$arch`, detached `.sig`
  signatures, `trustedkeys.gpg` keyring.
- **Configuration**: `/etc/xpm.conf` (TOML), user repos under `/etc/xpm.d/`,
  `$repo`/`$arch` URL variables, `sig_level` semantics.
- **Local database**: `/var/lib/xpm/local/<pkg>/` with `version`, `reason`,
  `origin`, `files`, `depends` and `provides` entries; sync DBs under
  `/var/lib/xpm/sync/`.
- **Scriptlet contract**: bash functions sourced from `.INSTALL` with
  `XPM_ROOT_DIR`, `XPM_PKG_NAME`, `XPM_PKG_VERSION` environment variables.

## See also

- Repo roadmap: `../../ROADMAP.md`
- xpm/xpkg integration: [`../INTEGRATION.md`](../INTEGRATION.md)
- Resolver architecture: [`../RESOLVER.md`](../RESOLVER.md)
