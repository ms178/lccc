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
# COMPATIBILITY WRAPPER: forwards to scripts/fuzz_diff.py --engine
# stress_suite, which drives gen_gep_chain_stress.py for seeds first..last
# and differentially evaluates every case against the reference at every
# requested optimisation level.  A case where GCC AND lccc die by the same
# signal is a generator bug: it is reported GEN-BUG and fails the run, the
# same tripwire the pre-consolidation script had (never a pass, never a
# silent skip).  Exit status is 1 on any MISCOMPILE / LCCC_CRASH / GEN-BUG.
#
# Usage: run_gep_chain_stress.sh [first-seed] [last-seed] [opt-levels...]
# Environment:
#   LCCC_BIN  compiler under test (default target/fastbuild/lccc)
#   GCC_BIN   oracle compiler     (default gcc)
# ============================================================================
set -euo pipefail

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
LCCC=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
GCC=${GCC_BIN:-gcc}

FIRST=${1:-1}
LAST=${2:-40}
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
    echo "gep-chain stress: empty seed range $FIRST..$LAST (nothing to do)"
    exit 0
fi

exec python3 "$REPO/scripts/fuzz_diff.py" \
    --engine stress_suite \
    --lccc "$LCCC" \
    --refs "$GCC" \
    --seed "$FIRST" \
    --count "$COUNT" \
    --opts="${OPTS// /,}"
