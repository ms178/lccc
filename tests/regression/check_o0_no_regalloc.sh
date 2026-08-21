#!/usr/bin/env bash
# RA-27: -O0 IR is non-SSA after phi elimination; it must use canonical stack homes.
set -euo pipefail
CCC=${CCC:-./target/release/lccc}; t=$(mktemp -d); trap 'rm -rf "$t"' EXIT
src=tests/regression/o0_phi_multidef.c
CCC_TRACE_ALLOCSTATS=kernel "$CCC" -O0 -S "$src" -o "$t/o0.s" 2>"$t/o0.log"
! grep -q '^\[RA-STATS\]' "$t/o0.log"
CCC_TRACE_ALLOCSTATS=kernel "$CCC" -O2 -S "$src" -o "$t/o2.s" 2>"$t/o2.log"
grep -q '^\[RA-STATS\] fn=kernel ' "$t/o2.log"
"$CCC" -O0 "$src" -o "$t/run"; "$t/run"
