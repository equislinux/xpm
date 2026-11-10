#!/usr/bin/env bash
# Local, network-free install benchmark: old xpm vs a new build.
#
# Builds a synthetic pacman-style repository (no dependencies), installs it
# into throwaway roots with each binary and reports wall times. It exercises
# metadata parsing, extraction and the parallel install pipeline without
# touching the host database.
#
# Usage:
#   scripts/bench-install-local.sh [--old BIN] [--new BIN] [--runs N]
#                                  [--packages N] [--workdir DIR]
#
# Defaults: old=/usr/bin/xpm, new=target/release/xpm, runs=3, packages=120.
set -euo pipefail

SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OLD="/usr/bin/xpm"
NEW="$SELF/target/release/xpm"
RUNS=3
PACKAGES=120
WORKDIR=""

usage() {
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --old) OLD="${2:?--old needs a path}"; shift 2 ;;
        --new) NEW="${2:?--new needs a path}"; shift 2 ;;
        --runs) RUNS="${2:?--runs needs a number}"; shift 2 ;;
        --packages) PACKAGES="${2:?--packages needs a number}"; shift 2 ;;
        --workdir) WORKDIR="${2:?--workdir needs a path}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "bench-install-local: unknown argument '$1'" >&2; exit 1 ;;
    esac
done

for cmd in bsdtar zstd repo-add; do
    command -v "$cmd" >/dev/null 2>&1 || {
        echo "bench-install-local: missing '$cmd'" >&2
        exit 1
    }
done
if [[ "$(id -u)" -ne 0 ]]; then
    command -v fakeroot >/dev/null 2>&1 || {
        echo "bench-install-local: missing 'fakeroot' (needed for a non-root install)" >&2
        exit 1
    }
fi
[[ -x "$OLD" ]] || { echo "bench-install-local: old binary not executable: $OLD" >&2; exit 1; }
[[ -x "$NEW" ]] || { echo "bench-install-local: new binary not executable: $NEW" >&2; exit 1; }

WORKDIR="${WORKDIR:-$(mktemp -d "${TMPDIR:-/tmp}/xpm-bench.XXXXXX")}"
mkdir -p "$WORKDIR/repo" "$WORKDIR/build"
trap 'rm -rf "$WORKDIR"' EXIT

run_as_root() {
    if [[ "$(id -u)" -eq 0 ]]; then
        "$@"
    else
        fakeroot -- "$@"
    fi
}

echo "== building $PACKAGES synthetic package(s)"
for ((i = 0; i < PACKAGES; i++)); do
    name="$(printf 'bench-pkg-%03d' "$i")"
    tree="$WORKDIR/build/$name"
    mkdir -p "$tree/usr/share/xpm-bench/$name"
    for ((j = 0; j < 32; j++)); do
        printf 'package %s file %s payload\n' "$name" "$j" > "$tree/usr/share/xpm-bench/$name/file-$j"
    done
    printf 'pkgname = %s\npkgver = 1.0-1\npkgdesc = benchmark %s\narch = x86_64\nsize = 4096\n' "$name" "$name" \
        > "$tree/.PKGINFO"
    ( cd "$tree" && bsdtar -cf - .PKGINFO usr | zstd -q -3 -o "$WORKDIR/repo/$name-1.0-1-x86_64.pkg.tar.zst" )
done
( cd "$WORKDIR/repo" && repo-add -q bench.db.tar.gz ./*.pkg.tar.zst >/dev/null )

cat > "$WORKDIR/xpm.conf" <<CONF
[options]
architecture = "x86_64"
sig_level = "never"
color = false
parallel_downloads = 10
shell_integration = false
mirror_pacman = false

[[repo]]
name = "bench"
server = ["file://$WORKDIR/repo"]
sig_level = "never"
CONF

package_args=()
for ((i = 0; i < PACKAGES; i++)); do
    package_args+=("$(printf 'bench-pkg-%03d' "$i")")
done

measure() {
    local label="$1" bin="$2" run_root="$3" cache="$4" start end
    rm -rf "$run_root" "$cache"
    mkdir -p "$run_root/db/sync" "$cache"
    cp "$WORKDIR/repo/bench.db.tar.gz" "$run_root/db/sync/bench.db"
    start=$(date +%s%N)
    if ! run_as_root "$bin" --config "$WORKDIR/xpm.conf" --root "$run_root" \
        --dbpath "$run_root/db" --cachedir "$cache" --no-confirm \
        --no-shell-integration --no-pacman-mirror --scriptlets=skip \
        -S "${package_args[@]}" > "$WORKDIR/$label.log" 2>&1; then
        echo "bench-install-local: $label failed; see $WORKDIR/$label.log" >&2
        tail -20 "$WORKDIR/$label.log" >&2
        exit 1
    fi
    end=$(date +%s%N)
    printf '%s\n' $(( (end - start) / 1000000 ))
}

old_times=()
new_times=()
for ((run = 1; run <= RUNS; run++)); do
    old_ms="$(measure "old-$run" "$OLD" "$WORKDIR/root-old" "$WORKDIR/cache-old" | tail -1)"
    new_ms="$(measure "new-$run" "$NEW" "$WORKDIR/root-new" "$WORKDIR/cache-new" | tail -1)"
    old_times+=("$old_ms")
    new_times+=("$new_ms")
    echo "run $run: old ${old_ms} ms | new ${new_ms} ms"
done

median() {
    printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END {if (NR % 2) print a[(NR+1)/2]; else print int((a[NR/2]+a[NR/2+1])/2)}'
}
old_med="$(median "${old_times[@]}")"
new_med="$(median "${new_times[@]}")"
echo
echo "median: old ${old_med} ms | new ${new_med} ms"
awk -v old="$old_med" -v new="$new_med" 'BEGIN { printf "speedup: %.2fx\n", old / new }'

old_files="$(find "$WORKDIR/root-old" -type f | wc -l)"
new_files="$(find "$WORKDIR/root-new" -type f | wc -l)"
echo "files: old $old_files | new $new_files"
if [[ "$old_files" != "$new_files" ]]; then
    echo "bench-install-local: file count mismatch" >&2
    exit 1
fi
