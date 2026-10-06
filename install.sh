#!/usr/bin/env bash
set -euo pipefail

XPM_PKG_URL_DEFAULT="https://equislinux.github.io/x-repo/x/x86_64/xpm-0.1.0-3-x86_64.xp"
XPM_PKG_URL="${XPM_PKG_URL:-$XPM_PKG_URL_DEFAULT}"
INSTALL_PREFIX="${INSTALL_PREFIX:-/usr/local}"
BIN_DEST="$INSTALL_PREFIX/bin/xpm"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command '$1' not found" >&2
    exit 1
  fi
}

download_file() {
  local url="$1"
  local out="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$out"
    return
  fi

  if command -v wget >/dev/null 2>&1; then
    wget -qO "$out" "$url"
    return
  fi

  echo "error: neither curl nor wget is available" >&2
  exit 1
}

main() {
  need_cmd tar
  need_cmd install

  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  local pkg="$tmp/xpm.xp"
  local rootfs="$tmp/rootfs"
  mkdir -p "$rootfs"

  echo "==> Downloading $XPM_PKG_URL"
  download_file "$XPM_PKG_URL" "$pkg"

  echo "==> Extracting package"
  tar --zstd -xpf "$pkg" -C "$rootfs"

  if [[ ! -f "$rootfs/usr/bin/xpm" ]]; then
    echo "error: package does not contain usr/bin/xpm" >&2
    exit 1
  fi

  echo "==> Installing to $BIN_DEST"
  if [[ "$(id -u)" -eq 0 ]]; then
    install -Dm755 "$rootfs/usr/bin/xpm" "$BIN_DEST"
  elif command -v sudo >/dev/null 2>&1; then
    sudo install -Dm755 "$rootfs/usr/bin/xpm" "$BIN_DEST"
  else
    echo "error: root privileges required (run as root or install sudo)" >&2
    exit 1
  fi

  echo "==> xpm installed successfully"
  echo "    version: $("$BIN_DEST" --version 2>/dev/null || echo unknown)"
  echo
  echo "Tip: set XPM_PKG_URL to install another build."
}

main "$@"
