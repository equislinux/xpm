#!/bin/sh
# Minimal pacman command shim for makepkg. It never calls pacman: package
# operations are delegated to xpm, while -Qi/-Qq read xpm's local database.
set -eu
XPM=/usr/local/bin/xpm
DB=/var/lib/xpm/local
ARCH=$(cat /etc/xpm-arch 2>/dev/null || uname -m)

list_pkgs() {
    if [ -d "$DB" ]; then
        find "$DB" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort
    fi
}

case "${1:-}" in
    -Qi)
        shift
        if [ "$#" -gt 0 ]; then
            names=$(printf '%s\n' "$@")
        else
            names=$(list_pkgs)
        fi
        printf '%s\n' "$names" | while IFS= read -r name; do
            [ -n "$name" ] || continue
            dir="$DB/$name"
            [ -d "$dir" ] || continue
            ver=$(cat "$dir/version" 2>/dev/null || true)
            [ -n "$ver" ] || continue
            printf 'Name            : %s\nVersion         : %s\nArchitecture    : %s\n\n' "$name" "$ver" "$ARCH"
        done
        ;;
    -Qq)
        list_pkgs
        ;;
    -T)
        shift
        exec "$XPM" -T "$@"
        ;;
    -S*|-U*|-R*|-Q*)
        exec "$XPM" "$@"
        ;;
    --version|-V)
        echo "xpm pacman compatibility shim"
        ;;
    *)
        exec "$XPM" "$@"
        ;;
esac
