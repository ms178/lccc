#!/usr/bin/env bash
# Codegen contract for named address spaces reached through a typedef or
# typeof (audit H2).
#
# tests/regression/seg_typedef/codegen_typedef_gs.c declares a __seg_gs object
# through a typedef, a typedef'd array, and typeof(). Every access must use the
# %gs: segment prefix. A compiler that drops the space emits plain memory, which
# is a silent miscompile. The test checks:
#   1. lccc emits exactly 4 %gs: memory operands for the 4 accesses;
#   2. no generic (un-prefixed) access to gobj or tobj remains;
#   3. GCC 14, the correctness oracle, emits the same number of %gs: operands.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
src=tests/regression/seg_typedef/codegen_typedef_gs.c
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-seg-typedef.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
. tests/regression/lib/gcc_oracle.sh
require_gcc_oracle "check_seg_typedef_codegen" || exit 1
fail=0

"$CCC" -S -O1 -o "$tmp/lccc.s" "$src"
"$GCC_ORACLE_BIN" -std=gnu11 -O1 -S -o "$tmp/gcc.s" "$src"

want=4
lccc_gs=$(grep -c '%gs:' "$tmp/lccc.s" || true)
gcc_gs=$(grep -c '%gs:' "$tmp/gcc.s" || true)
if [ "$lccc_gs" -ne "$want" ]; then
    echo "FAIL: lccc emitted $lccc_gs %gs: operands, want $want" >&2
    grep -n 'gobj\|garr\|tobj' "$tmp/lccc.s" >&2 || true
    fail=1
fi
if [ "$gcc_gs" -ne "$want" ]; then
    echo "FAIL: GCC $GCC_ORACLE_MAJOR emitted $gcc_gs %gs: operands, want $want (fixture expectation is wrong)" >&2
    fail=1
fi

# A generic access to a named-space symbol is a miscompile. The "(%rip)"
# form with no "%gs:" prefix is what the pre-fix output looked like.
if grep -nE '(^|[^:A-Za-z_])(gobj|tobj)\(%rip\)' "$tmp/lccc.s" >&2; then
    echo "FAIL: generic (non-%gs:) access to a __seg_gs object in lccc output" >&2
    fail=1
fi

if [ "$fail" -ne 0 ]; then
    echo "FAIL: check_seg_typedef_codegen" >&2
    exit 1
fi
echo "PASS: check_seg_typedef_codegen (lccc %gs: operands=$lccc_gs, GCC $GCC_ORACLE_MAJOR=$gcc_gs)"
