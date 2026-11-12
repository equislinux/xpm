#!/usr/bin/env bash
# Exercise the clean-chroot orchestration without bootstrapping a real root.
#
# The test creates a fake base-devel template and an `arch-chroot` stub that
# records every invocation and produces a fake package. It verifies the shared
# template, cheap copy, cache/source bind mounts, dependency installation,
# makepkg flags, artifact collection and work-directory cleanup.
#
# Run as root, or unprivileged on a kernel with unprivileged user namespaces
# (the script re-executes itself under `unshare -r -m`).
set -Eeuo pipefail

if [[ $EUID -ne 0 ]]; then
  if command -v unshare >/dev/null 2>&1 &&
     unshare -r -m true 2>/dev/null; then
    exec unshare -r -m bash "$0" "$@"
  fi
  echo "error: run as root or enable unprivileged user namespaces" >&2
  exit 1
fi

REPO_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
XPM_BIN=${XPM_BIN:-$REPO_DIR/target/debug/xpm}
if [[ ! -x "$XPM_BIN" ]]; then
  echo "error: $XPM_BIN is missing; run: cargo build -p xpm" >&2
  exit 1
fi

WORK=$(mktemp -d "${TMPDIR:-/tmp}/xpm-chroot-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

HARNESS_LOG=$WORK/log
export HARNESS_LOG
: > "$HARNESS_LOG"

BIN=$WORK/bin
BASE=$WORK/base
SRC=$WORK/src
PKGS=$WORK/pkgs
SRCDEST=$WORK/srcdest
BUILDDIR=$WORK/builddir
DB=$WORK/db
ARCH=$(uname -m)
TEMPLATE=$BASE/template-$ARCH
TPL_ROOT=$TEMPLATE/root

mkdir -p "$BIN" "$SRC" "$PKGS" "$SRCDEST" "$BUILDDIR" "$DB/sync" \
  "$TPL_ROOT/usr/bin" "$TPL_ROOT/var/lib/xpm/local" "$TPL_ROOT/etc"

printf '#!/bin/sh\n' > "$TPL_ROOT/usr/bin/bash"
printf '[options]\n' > "$TPL_ROOT/etc/xpm.conf"
XPM_VERSION=$("$XPM_BIN" --version | awk '{print $2}')
printf '%s\n' \
  "{\"schema\":1,\"arch\":\"$ARCH\",\"xpm_version\":\"$XPM_VERSION\",\"created_at\":$(date +%s),\"refreshed_at\":$(date +%s)}" \
  > "$TEMPLATE/template.json"
printf 'stale\n' > "$DB/sync/core.db"

cat > "$BIN/arch-chroot" <<'FAKE'
#!/bin/sh
set -eu
root=$1
shift
printf 'arch-chroot %s %s\n' "$root" "$*" >> "$HARNESS_LOG"
case " $* " in
  *" runuser "*)
    mkdir -p "$root/pkgdest"
    printf 'fake package\n' > "$root/pkgdest/demo-1.0-1-x86_64.pkg.tar.zst"
    ;;
esac
exit 0
FAKE
chmod +x "$BIN/arch-chroot"
export PATH="$BIN:$PATH"

cat > "$SRC/PKGBUILD" <<'PKGBUILD'
pkgname=demo
pkgver=1.0
pkgrel=1
arch=('x86_64')
PKGBUILD
cat > "$SRC/.SRCINFO" <<'SRCINFO'
pkgbase = demo
	pkgver = 1.0
	pkgrel = 1
	arch = x86_64
pkgname = demo
SRCINFO

# A stale root from a previous crash must disappear.
mkdir -p "$TEMPLATE/work-999-dead/root"
printf 'copy\n' > "$TEMPLATE/work-999-dead/.copy-kind"
touch "$TEMPLATE/work-999-dead/.lock"
chmod 644 "$TEMPLATE/work-999-dead/.lock"

printf 'fake preinstall\n' > "$WORK/preinstall.pkg.tar.zst"

run_build() {
  local pkgdest=$1
  "$XPM_BIN" build "$SRC" \
    --chroot --chroot-dir "$BASE" \
    --chroot-install gtk3 --chroot-install nss --nodeps \
    --preinstall "$WORK/preinstall.pkg.tar.zst" \
    --srcdest "$SRCDEST" --builddir "$BUILDDIR" \
    --pkgdest "$pkgdest" --json "$pkgdest/result.json"
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

run_build "$PKGS"
grep -q '"name": "demo"' "$PKGS/result.json" || fail "missing JSON artifact"
test -f "$PKGS/demo-1.0-1-x86_64.pkg.tar.zst" || fail "missing package file"
test ! -e "$TEMPLATE/work-999-dead" || fail "stale work directory was not removed"
test -d "$TPL_ROOT" || fail "template root was modified or removed"
test -f "$TEMPLATE/template.json" || fail "template stamp was removed"
grep -q 'gtk3 nss' "$HARNESS_LOG" || fail "chroot deps were not installed with xpm"
grep -q '/preinstall/preinstall.pkg.tar.zst' "$HARNESS_LOG" || fail "preinstall paths were not passed as files"
grep -q -- '--nodeps' "$HARNESS_LOG" || fail "makepkg did not receive --nodeps"
grep -q 'makepkg' "$HARNESS_LOG" || fail "makepkg was not invoked"
grep -q 'PKGEXT=.pkg.tar.zst' "$HARNESS_LOG" || fail "PKGEXT was not forced"
grep -q 'SRCDEST=/srcdest' "$HARNESS_LOG" || fail "SRCDEST was not forwarded"
grep -q 'BUILDDIR=/builddir' "$HARNESS_LOG" || fail "BUILDDIR was not forwarded"
test -z "$(find "$TEMPLATE" -maxdepth 1 -name 'work-*' -print -quit)" || fail "work directories left behind"

# The second build must reuse the same template without bootstrapping again.
before=$(grep -c 'runuser' "$HARNESS_LOG")
PKGS2=$WORK/pkgs2
mkdir -p "$PKGS2"
run_build "$PKGS2"
after=$(grep -c 'runuser' "$HARNESS_LOG")
[[ $after -eq $((before + 1)) ]] || fail "second build did not run exactly once"

echo "chroot orchestration: OK"
