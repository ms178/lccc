#!/usr/bin/env bash
# Bounded unsigned affine guards; wrapping entry must remain a refusal.
set -euo pipefail
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
python3 "$root/tests/regression/unsigned_affine_model.py"
for mode in -m64 -m32; do
    for opt in -O0 -O1 -O2 -O3 -Os; do
        "$GCC" "$mode" "$opt" -Wall -Wextra -Werror "$root/tests/regression/unsigned_affine_loop.c" -o "$tmp/ref"
        CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1 "$CCC" "$mode" "$opt" "$root/tests/regression/unsigned_affine_loop.c" -o "$tmp/mine"
        "$tmp/ref" > "$tmp/expected"
        "$tmp/mine" > "$tmp/actual"
        cmp "$tmp/expected" "$tmp/actual"
        if [[ $opt == -O2 || $opt == -O3 ]]; then
            CCC_NO_AFFINE_EXIT_FOLD=1 CCC_VERIFY_IR=abort CCC_VALIDATE_SSA=1 "$CCC" "$mode" "$opt" "$root/tests/regression/unsigned_affine_loop.c" -o "$tmp/off"
            "$tmp/off" > "$tmp/disabled"
            cmp "$tmp/expected" "$tmp/disabled"
        fi
    done
done
CCC_DEBUG_AFFINE_FOLD=1 "$CCC" -O2 -S "$root/tests/regression/unsigned_affine_loop.c" -o "$tmp/on.s" 2> "$tmp/on.log"
if ! grep -Eq '\[AFFOLD\] bounded_u64: affine exit-compare folds: [1-9]' "$tmp/on.log"; then
    echo 'FAIL: proved unsigned header guard did not fold' >&2
    exit 1
fi
# Exercise the retained late producer too; an earlier canonicalizer must not
# make its positive witness or the common kill switch vacuous.
CCC_DISABLE_PASSES=loop_bound CCC_DEBUG_AFFINE_FOLD=1 "$CCC" -O2 -S "$root/tests/regression/unsigned_affine_loop.c" -o "$tmp/late.s" 2> "$tmp/late.log"
if ! grep -Eq '\[AFFOLD\] bounded_u64: affine exit-compare folds: [1-9]' "$tmp/late.log"; then
    echo 'FAIL: retained late unsigned fold did not run with early placement disabled' >&2
    exit 1
fi
if ! grep -q '(early unsigned)' "$tmp/on.log"; then
    echo 'FAIL: early unsigned placement was not exercised' >&2
    exit 1
fi
CCC_NO_AFFINE_EXIT_FOLD=1 CCC_DEBUG_AFFINE_FOLD=1 "$CCC" -O2 -S "$root/tests/regression/unsigned_affine_loop.c" -o "$tmp/off.s" 2> "$tmp/off.log"
if grep -q '\[AFFOLD\]' "$tmp/off.log"; then
    echo 'FAIL: unsigned fold ignored its disable switch' >&2
    exit 1
fi
if grep -Eq '\[AFFOLD\] wraps_u(32|64):' "$tmp/on.log"; then
    echo 'FAIL: unsigned wraparound entry was folded' >&2
    exit 1
fi
echo 'PASS: unsigned affine m64/m32 runtime matrix, modular model, positive shape and wrap/disable controls'
