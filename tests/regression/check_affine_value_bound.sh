#!/usr/bin/env bash
# The VALUE-bounded affine exit-compare fold (`iv + C < n` -> `iv < n - C`
# with the subtraction hoisted behind the zero-trip guard) is an equivalence
# ONLY under its guard proof.  This gate pins four contracts:
#   1. bit-exact runtime parity with the GCC oracle, including the bounds
#      near LONG_MIN where an unguarded fold diverges;
#   2. the fold FIRES on the guarded signed and unsigned do-while shapes
#      (otherwise the gate is vacuous) and the hoisted subtraction reaches
#      the assembly;
#   3. the refused shapes -- non-strict compare, step 2, top-test loop --
#      produce NO value-bound fold;
#   4. CCC_NO_AFFINE_EXIT_FOLD still owns this producer (kill switch).
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_affine_value_bound: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_affine_value_bound: no gcc oracle" >&2; exit 1; }

corpus=$here/affine_value_bound.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-affvb.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# ---------------------------------------------------------------- contract 1
note "contract 1: runtime parity with the GCC oracle (fold on and killed)"
"$GCC" -O2 "$corpus" -o "$work/ref" || bad "gcc oracle failed to build"
expected=$(timeout 60 "$work/ref") || bad "gcc oracle exited non-zero"
case $expected in
    *"affine-value-bound: all shapes exact"*) ;;
    *) bad "gcc oracle did not report the expected success line" ;;
esac
for opt in O1 O2 O3 Os; do
    "$CCC" "-$opt" "$corpus" -o "$work/on-$opt" \
        || { bad "-$opt: lccc failed to build"; continue; }
    out=$(timeout 60 "$work/on-$opt") || { bad "-$opt: exited non-zero"; continue; }
    [[ $out == "$expected" ]] || bad "-$opt: stdout differs from the gcc oracle"
done
env CCC_NO_AFFINE_EXIT_FOLD=1 "$CCC" -O2 "$corpus" -o "$work/off" \
    || bad "killed build failed"
out=$(timeout 60 "$work/off") || bad "killed run exited non-zero"
[[ $out == "$expected" ]] || bad "killed: stdout differs from the gcc oracle"

# ---------------------------------------------------------------- contract 2
note "contract 2: the fold fires on the guarded shapes and hoists the sub"
env CCC_DEBUG_AFFINE_FOLD=1 "$CCC" -O2 -S "$corpus" -o "$work/on.s" \
    2> "$work/dbg.log" || bad "debug build failed"
folds=$(grep -c 'value-bound fold' "$work/dbg.log" || true)
[[ $folds -eq 3 ]] || bad "expected exactly 3 value-bound folds (sum_dw, scan_dw_u, toptest), got $folds"
grep -q 'sum_dw: value-bound fold' "$work/dbg.log" || bad "signed shape did not fold"
grep -q 'scan_dw_u: value-bound fold' "$work/dbg.log" || bad "unsigned shape did not fold"
# The plain for loop folds at inversion time, against the duplicated test.
grep -q 'toptest: value-bound fold (inverted latch)' "$work/dbg.log" \
    || bad "top-test for loop did not fold its inverted latch"
# The hot latch must compare against the pre-folded bound: inside sum_dw
# there is a `subq $4` before the loop and NO per-trip `leaq 4(` remains.
sum_asm=$(awk '/^sum_dw:/{f=1} f{print} f&&/cfi_endproc/{exit}' "$work/on.s")
grep -q 'subq \$4' <<<"$sum_asm" || bad "hoisted 'subq \$4' missing from sum_dw"
if grep -q 'leaq 4(' <<<"$sum_asm"; then
    bad "per-trip 'leaq 4(...)' survived in sum_dw"
fi

# ---------------------------------------------------------------- contract 3
note "contract 3: refused shapes do not fold"
for f in le_dw step2_dw reentry; do
    if grep -q "$f: value-bound fold" "$work/dbg.log"; then
        bad "$f must refuse the value-bound fold"
    fi
done

# ---------------------------------------------------------------- contract 4
note "contract 4: CCC_NO_AFFINE_EXIT_FOLD owns the producer"
env CCC_NO_AFFINE_EXIT_FOLD=1 CCC_DEBUG_AFFINE_FOLD=1 "$CCC" -O2 -S "$corpus" \
    -o "$work/off.s" 2> "$work/off.log" || bad "killed debug build failed"
if grep -q 'value-bound fold' "$work/off.log"; then
    bad "kill switch left the value-bound fold running"
fi

if [[ $fail -ne 0 ]]; then
    echo "check_affine_value_bound: FAIL" >&2
    exit 1
fi
echo "check_affine_value_bound: OK"
