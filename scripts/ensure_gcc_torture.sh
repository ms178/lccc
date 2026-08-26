#!/usr/bin/env bash
# ensure_gcc_torture.sh — (re)provision the GCC C torture/execute corpus after
# a harness wipe.
#
# The Arena workspace snapshot caps at ~10k files, but the extracted GCC
# testsuite (gcc.c-torture + gcc.dg) is ~22k files, so it never survives a
# restore intact. The upstream tarball under /home/user/dl does survive (a
# single 98 MB file), so this script re-extracts only the two needed
# subtrees idempotently:
#
#   /home/user/src/gcc/gcc/testsuite/gcc.c-torture/execute  (torture corpus)
#   /home/user/src/gcc/gcc/testsuite/gcc.dg                 (dg/torture headers)
#
# Usage:  scripts/ensure_gcc_torture.sh [tarball] [dest-root]
set -euo pipefail

tarball=${1:-/home/user/dl/gcc-15.1.0.tar.xz}
dest=${2:-/home/user/src/gcc/gcc/testsuite}
ver=$(basename "$tarball" .tar.xz)

if [[ ! -f "$tarball" ]]; then
    mkdir -p "$(dirname "$tarball")"
    curl -sSLo "$tarball" "https://ftp.gnu.org/gnu/gcc/${ver}/${ver}.tar.xz"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
tar -xJf "$tarball" -C "$tmp" "${ver}/gcc/testsuite/gcc.c-torture" "${ver}/gcc/testsuite/gcc.dg"

mkdir -p "$dest"
rm -rf "$dest/gcc.c-torture" "$dest/gcc.dg"
mv "${tmp}/${ver}/gcc/testsuite/gcc.c-torture" "$dest/"
mv "${tmp}/${ver}/gcc/testsuite/gcc.dg" "$dest/"

n=$(ls "$dest/gcc.c-torture/execute"/*.c 2>/dev/null | wc -l)
echo "gcc.c-torture/execute: ${n} sources at $dest/gcc.c-torture/execute"
