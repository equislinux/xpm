#!/usr/bin/env bash
# Local CI for xpm. No network access is required for the build/CLI checks.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "== fmt =="
cargo fmt --all --check

echo "== clippy =="
cargo clippy --all-targets -- -D warnings

echo "== tests =="
cargo test --workspace

echo "== release build =="
cargo build --release -p xpm

echo "== xpm build fixture =="
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/pkg"
cat > "$TMP/pkg/PKGBUILD" <<'PKG'
pkgname=xpm-local-ci-fixture
pkgver=1.0
pkgrel=1
pkgdesc="xpm local CI fixture"
arch=('any')
license=('MIT')
source=()
package() {
  install -Dm644 /dev/null "$pkgdir/usr/share/xpm-local-ci-fixture/marker"
}
PKG
"$ROOT/target/release/xpm" build --json "$TMP/result.json" "$TMP/pkg" >/dev/null
test -s "$TMP/result.json"
grep -q 'xpm-local-ci-fixture' "$TMP/result.json"
test -f "$TMP/pkg/.xpm-packages"/xpm-local-ci-fixture-1.0-1-any.pkg.tar.zst
echo "local CI OK"
