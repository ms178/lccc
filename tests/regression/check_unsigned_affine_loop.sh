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
# The early range-proof canonicalizer and late affine rotation are independent
# mechanisms. Exercise all four actual configurations, not a coupled switch.
for early in on off; do
    for late in on off; do
        flags=()
        [[ $early != off ]] || flags+=(CCC_DISABLE_PASSES=loop_bound)
        [[ $late != off ]] || flags+=(CCC_NO_AFFINE_EXIT_FOLD=1)
        stem="$tmp/early-$early-late-$late"
        env -u CCC_DISABLE_PASSES -u CCC_NO_AFFINE_EXIT_FOLD \
            CCC_DEBUG_AFFINE_FOLD=1 "${flags[@]}" "$CCC" -O2 -S \
            "$root/tests/regression/unsigned_affine_loop.c" -o "$stem.s" 2> "$stem.log"
        if [[ $early == on ]]; then
            grep -Eq '\[AFFOLD\] bounded_u64:.*\(early unsigned\)' "$stem.log"
        elif grep -q '(early unsigned)' "$stem.log"; then
            echo 'FAIL: independent early disable was ignored' >&2; exit 1
        elif [[ $late == on ]]; then
            grep -Eq '\[AFFOLD\] bounded_u64: affine exit-compare folds: [1-9]' "$stem.log"
        fi
        if [[ $early == off && $late == off ]] && grep -q '\[AFFOLD\]' "$stem.log"; then
            echo 'FAIL: both fold mechanisms were disabled but a fold ran' >&2; exit 1
        fi
        env -u CCC_DISABLE_PASSES -u CCC_NO_AFFINE_EXIT_FOLD \
            "${flags[@]}" "$CCC" -O2 -fwrapv \
            "$root/tests/regression/unsigned_affine_loop.c" -o "$stem"
        "$stem" > "$stem.stdout"
        cmp "$tmp/expected" "$stem.stdout"
    done
done
if grep -Eq '\[AFFOLD\] wraps_u(32|64):' "$tmp/on.log"; then
    echo 'FAIL: unsigned wraparound entry was folded' >&2
    exit 1
fi
echo 'PASS: unsigned affine m64/m32 runtime matrix, modular model, positive shape, wrap controls and independent 2x2 disables'
