# X package stack independence roadmap

Goal: use Arch repositories and AUR without invoking or linking pacman, while
keeping a compatibility layer for the ecosystem that still expects pacman.

## Current baseline

- [x] xpm owns install/remove/upgrade transactions for the running system.
- [x] xpm parses Arch sync databases and verifies signatures/checksums.
- [x] xpm-aur resolves AUR metadata and installs official packages with xpm.
- [x] AUR clean builds share one `base-devel` template and isolate each build
      in a disposable copy.
- [x] Signed remote payloads exist for post-install provisioning.
- [x] xpm build chroots require only `arch-chroot`; `pacstrap` is not used.
- [x] xpm-aur installs build dependencies with xpm and runs `makepkg --nodeps`.
- [x] The pacman shim covers `/usr/local/bin/pacman` and `/usr/bin/pacman`
      inside disposable roots.
- [ ] `libalpm` is not used (and should not be required for normal operation).

## P0 — Remove pacman from the build bootstrap

- [x] Add `xpm bootstrap --root DIR [packages...]`.
- [x] Bootstrap an empty root with xpm: sync, filesystem, bash, then the rest.
- [x] Use the host pacman keyring only for verification; never run pacman.
- [x] Make `xpm build --chroot` use xpm bootstrap instead of `pacstrap`.
- [x] Replace `pacman -U /preinstall/*` with xpm local-package install.
- [x] Cache/reuse the bootstrap root; avoid creating base-devel per AUR base.
- [x] Test a clean root build as a non-root xpm-aur user.
- [ ] Test a clean root build with no pacman binary in PATH; xpm still needs a compatibility shim for makepkg.
- [x] Fix shim installation permissions for non-root builds; the shim/xpm are installed into the root-owned chroot copy with sudo.
- [x] Create `/proc`, `/sys`, `/dev`, `/run` and `/tmp` in the bootstrap/chroot root before arch-chroot.
- [ ] Remove the remaining makepkg pacman call seen while generating `.BUILDINFO`.

## P1 — AUR build dependencies without pacman

Status: `xpm bootstrap` and the chroot bootstrap are implemented. A test build
created a clean base-devel root with xpm, but `makepkg` still invoked pacman
while generating `.BUILDINFO`; that is the next concrete blocker.

- [x] Run `makepkg --nodeps` in the chroot.
- [x] Resolve `depends`, `makedepends` and `checkdepends` from AUR metadata.
- [x] Install official build dependencies into the chroot with xpm.
- [x] Install AUR dependency artifacts with xpm, preserving dependency order.
- [ ] Keep split-package artifacts and preinstall ordering deterministic.
- [ ] Detect and report packages whose PKGBUILD still calls pacman.

## P2 — Make and package-manager compatibility

- [x] Add a minimal `pacman` shim for makepkg (`-Qi`, `-Qq`, `-T`, `-S`, `-U`, `-R`); validate against a clean build.
- [ ] Cover the commands used by makepkg and common AUR scripts (`-Q`, `-T`,
      `-Qq`, `-Qm`, `-U`, `-S`, `-R`, `-Sy`, `-Syu`, `--version`).
- [ ] Keep pacman's local DB and keyring readable/writable for interoperability.
- [ ] Implement or validate `provides`, `replaces`, `conflicts`, `groups`,
      hooks and scriptlet semantics against pacman behavior.
- [ ] Add an audit test that fails if new code shells out to pacman.

## P3 — Full independence

- [ ] Replace `makepkg` with an xpm-native PKGBUILD builder (or vendor it).
- [ ] Implement a native keyring/bootstrap path without `pacman-key`.
- [ ] Verify repo DB sync, signatures, package files and local DB across
      pacman/xpm handoff.
- [ ] Define the X base without a runtime pacman dependency (or keep it only as
      an optional compatibility package).
- [ ] Validate common AUR packages, split packages, VCS packages and hooks.

## Definition of done for "100% independent"

- A full install and AUR build can run with no pacman executable and no
  libalpm linked.
- Arch repositories still work by URL and database format.
- AUR still works through metadata, PKGBUILDs and built package artifacts.
- Compatibility shims are optional, not required by xpm's own flow.
- Audit: `strace`/tests show no `pacman` or `libalpm` use in normal paths.
