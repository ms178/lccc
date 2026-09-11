#!/usr/bin/env bash
# IS-01 / folded-index RA contract: the masked CRC table index must retain a
# home through its widening cast so x86-64 can emit one SIB table load.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
tmp=${TMPDIR:-/tmp}/lccc-crc-index.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"
source_file="$root/tests/benchmark/programs/gzip_crc32.c"
"$CCC" -O2 -march=x86-64-v3 -S "$source_file" -o "$tmp/t.s"
"$CCC" -O2 -march=x86-64-v3 "$source_file" -o "$tmp/t"
[[ $("$tmp/t") == 372e56ab ]]
if ! grep -Eq 'movl \(%r[a-z0-9]+, %r[a-z0-9]+, 4\), %' "$tmp/t.s"; then
    echo "CRC table load did not use a scale-4 SIB operand" >&2
    exit 1
fi
