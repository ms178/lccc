#!/usr/bin/env bash
# ============================================================================
# run_gep_chain_stress.sh — differential stress test for FOLDED-ADDRESS
#                           (chained-GEP) liveness.
#
# Generates deterministic C programs with scripts/gen_gep_chain_stress.py —
# a caller that holds a pointer which is the ROOT of a 2- or 3-link
# constant-offset GEP chain, an inlined callee that burns registers in a loop
# before it reads the folded fields, and (variably) a caller-side branch that
# puts a hole in the root's live range — then requires lccc to produce
# byte-identical output to GCC for every case at every optimisation level.
#
# This is the bug class of the sqlite3.50 -O1 SIGSEGV: the backend composes
# GEP(GEP(p,+a),+b) into `a+b(%p)`, so `p` is read at the Load/Store, but the
# IR records its last use at the folded-away GEP.  Liveness must extend the
# root to every folded access; if it does not, the register allocator recycles
# `p`'s register inside the inlined body and the access dereferences garbage.
#
# Usage: run_gep_chain_stress.sh [first-seed] [last-seed] [opt-levels...]
# Environment:
#   LCCC_BIN  compiler under test (default target/fastbuild/lccc)
#   GCC_BIN   oracle compiler     (default gcc)
# ============================================================================
set -uo pipefail

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
LCCC=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
FIRST=${1:-1}
LAST=${2:-40}
shift 2 2>/dev/null || true
OPTS=${*:--O1 -O2 -O3 -Os}

WORK=$(mktemp -d /tmp/gepchain-stress.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; skip=0
failed_cases=()

for seed in $(seq "$FIRST" "$LAST"); do
    "$REPO/scripts/gen_gep_chain_stress.py" "$seed" > "$WORK/case.c" || continue
    for opt in $OPTS; do
        if ! $GCC "$opt" "$WORK/case.c" -o "$WORK/ref" >"$WORK/gcc.err" 2>&1; then
            skip=$((skip+1)); echo "SKIP seed=$seed $opt (gcc rejected the case)"
            continue
        fi
        "$WORK/ref" > "$WORK/ref.out" 2>&1
        rc_ref=$?
        if ! $LCCC "$opt" "$WORK/case.c" -o "$WORK/tst" >"$WORK/lccc.err" 2>&1; then
            fail=$((fail+1))
            failed_cases+=("seed=$seed $opt (compile)")
            echo "FAIL seed=$seed $opt: lccc compile error"
            head -5 "$WORK/lccc.err"
            continue
        fi
        "$WORK/tst" > "$WORK/tst.out" 2>&1
        rc_tst=$?
        if [ "$rc_ref" -eq 139 ] && [ "$rc_tst" -eq 139 ]; then
            # Both segfault: a generator bug, not a compiler bug. Never a pass.
            fail=$((fail+1))
            failed_cases+=("seed=$seed $opt (both SIGSEGV: generator bug)")
            echo "FAIL seed=$seed $opt: both SIGSEGV (generator bug)"
        elif ! cmp -s "$WORK/ref.out" "$WORK/tst.out" || [ "$rc_ref" -ne "$rc_tst" ]; then
            fail=$((fail+1))
            failed_cases+=("seed=$seed $opt")
            echo "FAIL seed=$seed $opt (rc ref=$rc_ref tst=$rc_tst)"
            diff "$WORK/ref.out" "$WORK/tst.out" | head -6
        else
            pass=$((pass+1))
        fi
    done
done

echo "================================================================"
echo "gep-chain stress: PASS=$pass FAIL=$fail SKIP=$skip"
if [ "$fail" -gt 0 ]; then
    printf 'failed: %s\n' "${failed_cases[*]}"
fi
echo "================================================================"
[ "$fail" -eq 0 ]
