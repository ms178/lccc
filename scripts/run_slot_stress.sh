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
#   default | CCC_NO_TIER2_GRAPH=1 | CCC_NO_SMALL_SLOTS=1 | both disabled
#   x -O0 -O1 -O2 -O3 -Os
#
# The four-way A/B is the point: a stack layout that is only correct when
# slot sharing is disabled is a miscompile waiting for the right TU. Tier-2
# (liveness-packed) slot sharing and 4-byte small slots are BOTH ON by
# default; CCC_NO_TIER2_GRAPH=1 and CCC_NO_SMALL_SLOTS=1 each disable one
# dimension, and setting both disables the two optimisations together.
# (CCC_TIER2_GRAPH=1 is accepted but is a historical no-op alias — Tier-2 is
# the default configuration now.)
#
# COMPATIBILITY WRAPPER: forwards to scripts/fuzz_diff.py --engine
# stress_suite, which drives gen_slot_stress.py for seeds first..last and
# evaluates every generated case under the four-way CCC layout matrix declared
# via the repeatable --config-env axes (both compilers, compile and run, under
# each combination).  A case where GCC AND lccc die by the same signal is a
# generator bug (GEN-BUG, a failure — never a pass).  Exit status is 1 on any
# MISCOMPILE / LCCC_CRASH / GEN-BUG.
#
# Usage: run_slot_stress.sh [first-seed] [last-seed] [opt-levels...]
# ============================================================================
set -euo pipefail

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
LCCC=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}

FIRST=${1:-1}
# Default range 1..45: the pre-existing -O2 VLA-store miscompile documented in
# engineering/BUG-2026-09-03-O2-vla-store-miscompile.md (seed 32) is FIXED
# (wide-imm memory-relay operand-size hardening), so the gate now covers the
# range that used to expose it.  Verified 720/720 across seeds 1..45 at
# -O1/-O2/-O3/-Os in all four layout configurations.
LAST=${2:-45}
if [[ $# -ge 2 ]]; then
    shift 2
else
    shift $# 2>/dev/null || true
fi
OPTS=${*:--O1 -O2 -O3 -Os}

COUNT=$(( LAST - FIRST + 1 ))
if (( COUNT < 1 )); then
    # Matches the pre-consolidation `seq first last` behaviour: an empty seed
    # range runs zero cases and succeeds.
    echo "slot stress: empty seed range $FIRST..$LAST (nothing to do)"
    exit 0
fi

exec python3 "$REPO/scripts/fuzz_diff.py" \
    --engine stress_suite \
    --lccc "$LCCC" \
    --refs "$GCC" \
    --seed "$FIRST" \
    --count "$COUNT" \
    --opts="${OPTS// /,}" \
    --config-env CCC_NO_TIER2_GRAPH=1 \
    --config-env CCC_NO_SMALL_SLOTS=1
