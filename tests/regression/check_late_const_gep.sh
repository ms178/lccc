#!/usr/bin/env bash
# A14-30: a dominating constant GEP can follow its consumer in block-vector
# emission order. Both constant and indexed maps must gate typed MachInst
# memory lowering; changing IV widening or disabling MachInst is not a fix.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-late-const-gep.XXXXXXXX")
trap 'rm -rf "$work"' EXIT
small="$root/tests/regression/fixtures/late_const_gep"
full="$root/engineering/evidence/2026-10-14-audit/zlib-maketrees"
"$GCC" -O2 -c "$small/check.c" -o "$work/check.o"
"$GCC" -O2 "$small/main.c" "$work/check.o" -o "$work/reference"
"$work/reference" > "$work/expected"
for opt in O0 O1 O2 O3 Os; do
    "$CCC" "-$opt" -c "$small/main.c" -o "$work/main.o"
    "$GCC" "$work/main.o" "$work/check.o" -o "$work/small"
    "$work/small" > "$work/actual" || { echo "FAIL late-constant-GEP -$opt" >&2; exit 1; }
    cmp "$work/expected" "$work/actual"
done
# Compile both complete zlib-derived TUs with LCCC. The expected bytes were
# established independently with GCC, Clang and GCC ASan/UBSan (see README).
# This is no longer a quarantined known-failure: require the exact full output.
for opt in O0 O1 O2 O3 Os; do
    "$CCC" "-$opt" "$full/maketrees-small.c" "$full/gencodes-small.c" -o "$work/full"
    "$work/full" > "$work/tables"
    cmp "$full/expected.txt" "$work/tables"
done
# Same optimized codegen, independent assembler/linker.
"$CCC" -O2 -S "$small/main.c" -o "$work/main.s"
"$GCC" "$work/main.s" "$work/check.o" -o "$work/gas"
"$work/gas" > "$work/actual"
cmp "$work/expected" "$work/actual"
echo 'OK late-constant-GEP: small + zlib-derived two-TU fixtures O0/O1/O2/O3/Os; GAS control'
