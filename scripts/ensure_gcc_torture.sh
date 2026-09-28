#!/usr/bin/env bash
# ensure_gcc_torture.sh — (re)provision the GCC C torture corpus after a
# harness wipe.
#
# The Arena workspace snapshot caps at ~128 MB / ~10k files, and the extracted
# GCC testsuite (gcc.c-torture + gcc.dg) is ~22k files, while the upstream
# tarball alone is ~107 MB.  Neither may live in the persisted zone: either
# one pushes the snapshot over its cap and the harness then drops the WHOLE
# snapshot (repository included).  Both therefore live under the
# snapshot-excluded `.cache` directory and are re-derived on demand — the
# download is ~20 s, the selective extraction ~60 s on the 2-core host.
#
#   $LCCC_DL/gcc-<ver>.tar.xz                          (download cache)
#   $dest/gcc.c-torture/{execute,compile}              (torture corpus)
#   $dest/gcc.dg                                       (dg/torture headers)
#
# Idempotent: when $dest/.lccc-provisioned names the requested version the
# script only reports the corpus size.  Pass --force to re-extract.
#
# Usage:  scripts/ensure_gcc_torture.sh [--force] [tarball] [dest-root]
# Env:    GCC_TORTURE_VERSION (default 16.2.0), LCCC_DL (download cache dir),
#         GCC_TESTSUITE_ROOT (default dest-root)
#
# The x86 torture runner (scripts/x86_gcc_torture.py) resolves the same
# default location, so `ensure_gcc_torture.sh && x86_gcc_torture.py` works
# from a freshly wiped sandbox without any environment.
set -euo pipefail

force=0
if [[ ${1:-} == --force ]]; then
    force=1
    shift
fi

ver_num=${GCC_TORTURE_VERSION:-16.2.0}
dl=${LCCC_DL:-/home/user/.cache/lccc-dl}
tarball=${1:-$dl/gcc-${ver_num}.tar.xz}
dest=${2:-${GCC_TESTSUITE_ROOT:-/home/user/.cache/lccc-gcc-testsuite}}
ver=$(basename "$tarball" .tar.xz)
stamp="$dest/.lccc-provisioned"

report() {
    local n_exec n_comp
    n_exec=$(find "$dest/gcc.c-torture/execute" -maxdepth 1 -name '*.c' 2>/dev/null | wc -l)
    n_comp=$(find "$dest/gcc.c-torture/compile" -maxdepth 1 -name '*.c' 2>/dev/null | wc -l)
    echo "gcc.c-torture ($ver): execute=${n_exec} compile=${n_comp} sources under $dest/gcc.c-torture"
}

if [[ $force -eq 0 && -f "$stamp" && "$(cat "$stamp")" == "$ver" ]]; then
    report
    exit 0
fi

if [[ ! -s "$tarball" ]]; then
    mkdir -p "$(dirname "$tarball")"
    # Download off-path and rename: a wipe/kill mid-transfer must never leave
    # a truncated tarball that a later run would trust.
    curl -fsSLo "$tarball.part" "https://ftp.gnu.org/gnu/gcc/${ver}/${ver}.tar.xz"
    mv -f "$tarball.part" "$tarball"
fi

mkdir -p "$(dirname "$dest")"
tmp=$(mktemp -d "$(dirname "$dest")/.torture-extract.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
tar -xJf "$tarball" -C "$tmp" "${ver}/gcc/testsuite/gcc.c-torture" "${ver}/gcc/testsuite/gcc.dg"

mkdir -p "$dest"
rm -rf "$dest/gcc.c-torture" "$dest/gcc.dg"
mv "${tmp}/${ver}/gcc/testsuite/gcc.c-torture" "$dest/"
mv "${tmp}/${ver}/gcc/testsuite/gcc.dg" "$dest/"
printf '%s\n' "$ver" > "$stamp"

report
