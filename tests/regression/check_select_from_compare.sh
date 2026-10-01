#!/usr/bin/env bash
# ============================================================================
# check_select_from_compare.sh — the select-from-compare fusion.
#
# `flags_peepholes::fold_setcc_test_cmov` collapses the backend's
# `setcc; (widen); test; cmov` idiom into `cmovcc` on the ORIGINAL comparison's
# flags.  Contracts:
#
#   1. SEMANTICS.  select_from_compare_fusion.c must agree with the GCC oracle
#      byte for byte at -O2/-O3/-O1 (the fusion is a peephole; the shapes are
#      chosen so it applies at every tier) and with the kill switch set.
#
#   2. THE FUSION FIRES.  In the byte-fold kernel the emitted code must branch
#      on the comparison directly: `cmovbe` present, `setbe` absent (the
#      boolean definition), and no `testl %.., %..` self-test relay.  Measured
#      cost before this was recognised: 2,522,590 Ir vs 2,211,009 after on
#      tests/benchmark/programs/ascii_case_fold.c (Callgrind, -O2
#      -march=x86-64-v3).
#
#   3. THE REFUSALS STAY EXACT.  `bool_kept` (the boolean has a second use) and
#      `sign_bit` (a 16-bit widening of a family whose high byte the `setcc`
#      never defined) are covered by contract 1's differential; contract 3
#      additionally requires that the second use survives -- the fused branch
#      must not have eaten a live definition.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)
CCC=${CCC:-$root/target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_select_from_compare: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_select_from_compare: no gcc oracle" >&2; exit 1; }

corpus=$here/select_from_compare_fusion.c
bench=$root/tests/benchmark/programs/ascii_case_fold.c
work=$(mktemp -d "${TMPDIR:-/tmp}/lccc-sfc.XXXXXX")
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

note "contract 1: oracle parity across opt levels and the kill switch"
"$GCC" -O2 "$corpus" -o "$work/ref" || bad "gcc oracle failed to build"
expected=$(timeout 300 "$work/ref") || bad "gcc oracle exited non-zero"

for opt in -O1 -O2 -O3; do
    for envs in "" "CCC_NO_FLAG_PEEPHOLES=1"; do
        name="$(echo "opt$opt${envs:+killed}" | tr -d ' -')"
        if ! "$CCC" "$opt" "$corpus" -o "$work/$name" > "$work/$name.build" 2>&1; then
            bad "$opt ${envs:+killed}: lccc failed to build"; continue
        fi
        local_out=$(timeout 300 env $envs "$work/$name") || { bad "$opt ${envs:+killed}: non-zero exit"; continue; }
        [[ $local_out == "$expected" ]] || bad "$opt ${envs:+killed}: stdout differs from the oracle"
    done
done
note "parity: -O1/-O2/-O3 x {default, kill switch}"

note "contract 2: the fused cmov consumes the comparison, on the bench kernel"
"$CCC" -O2 -march=x86-64-v3 -S "$bench" -o "$work/bench.s" || bad "-S build failed"
body() { awk '/^main:/ { inside=1 } inside { print } inside && /\.size/ { exit }' "$1"; }
bench_body=$(body "$work/bench.s")
grep -qE "^[[:space:]]+cmovbe" <<< "$bench_body" \
    || bad "no \`cmovbe\` in main: the byte-range select is not fused"
grep -qE "^[[:space:]]+setbe[[:space:]]" <<< "$bench_body" \
    && bad "a \`setbe\` relay survived: the comparison is still materialised as a byte"
grep -qE "^[[:space:]]+testl[[:space:]]" <<< "$bench_body" \
    && bad "a \`testl\` self-test relay survived in the fused loop"

"$CCC" -O2 -march=x86-64-v3 -S "$corpus" -o "$work/corpus.s" || bad "-S build failed (corpus)"
kept=$(awk '/^bool_kept:/ { inside=1 } inside { print } inside && /\.size/ { exit }' "$work/corpus.s")
grep -qE "set(g|l|le|a|ae|b|be)[[:space:]]" <<< "$kept" \
    || bad "bool_kept: the second use of the boolean lost its definition (no setcc left)"

if [[ $fail -ne 0 ]]; then
    echo "check_select_from_compare: FAILED" >&2
    exit 1
fi
echo "check_select_from_compare: PASS (oracle parity at -O1/-O2/-O3 x kill switch, fused cmov on the bench kernel, live boolean kept)"
