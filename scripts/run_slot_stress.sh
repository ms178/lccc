#!/usr/bin/env bash
# ============================================================================
# run_slot_stress.sh — differential stress test for stack-slot allocation.
#
# Generates deterministic C programs with scripts/gen_slot_stress.py (many
# simultaneously live values of mixed widths across diamonds, loops, switches,
# call barriers, address-taken/volatile/alloca/inline-asm/setjmp/VLA values),
# then requires lccc to produce byte-identical output to GCC for every case in
# every configuration:
#
#   default | CCC_NO_SMALL_SLOTS=1 | CCC_TIER2_GRAPH=1 | both
#   x -O0 -O1 -O2 -O3 -Os
#
# The three-way A/B is the point: a stack layout that is only correct when
# slot sharing is disabled is a miscompile waiting for the right TU. This is
# the harness that must pass before Tier-2 (liveness-packed) slot sharing can
# be on by default.
#
# Usage: run_slot_stress.sh [first-seed] [last-seed] [opt-levels...]
# ============================================================================
set -uo pipefail

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
LCCC=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}
FIRST=${1:-1}
LAST=${2:-20}
shift 2 2>/dev/null || true
OPTS=${*:-"-O1 -O2 -O3 -Os"}

WORK=$(mktemp -d /tmp/slot-stress.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

pass=0; fail=0; skip=0
failed_cases=()

for seed in $(seq "$FIRST" "$LAST"); do
    "$REPO/scripts/gen_slot_stress.py" "$seed" > "$WORK/case.c" || continue
    for opt in $OPTS; do
        if ! $GCC "$opt" "$WORK/case.c" -o "$WORK/ref" >"$WORK/gcc.err" 2>&1; then
            skip=$((skip+1)); echo "SKIP seed=$seed $opt (gcc rejected the case)"; continue
        fi
        expected=$("$WORK/ref" 2>&1); exp_rc=$?
        while IFS='|' read -r label envs; do
            out=$(env $envs "$LCCC" "$opt" "$WORK/case.c" -o "$WORK/lcc" 2>"$WORK/lccc.err")
            if [[ $? -ne 0 ]]; then
                fail=$((fail+1)); failed_cases+=("seed=$seed $opt $label: COMPILE FAIL")
                echo "FAIL seed=$seed $opt $label: compiler error: $(head -2 "$WORK/lccc.err" | tr '\n' ' ')"
                continue
            fi
            got=$("$WORK/lcc" 2>&1); rc=$?
            if [[ "$got" != "$expected" || $rc -ne $exp_rc ]]; then
                fail=$((fail+1)); failed_cases+=("seed=$seed $opt $label")
                echo "FAIL seed=$seed $opt $label: got '$got' (rc=$rc) want '$expected' (rc=$exp_rc)"
            else
                pass=$((pass+1))
            fi
        done <<'CFG'
default|LCCC_NO_SMALL_SLOTS=0
no-small-slots|CCC_NO_SMALL_SLOTS=1
tier2-on|CCC_TIER2_GRAPH=1
tier2-on+no-small|CCC_TIER2_GRAPH=1 CCC_NO_SMALL_SLOTS=1
CFG
    done
done

echo "================================================================"
echo "slot stress: PASS=$pass FAIL=$fail SKIP=$skip  (seeds $FIRST..$LAST, opts: $OPTS)"
if [[ $fail -gt 0 ]]; then
    printf 'failing case: %s\n' "${failed_cases[@]}"
    exit 1
fi
exit 0
