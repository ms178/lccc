#!/usr/bin/env bash
# ============================================================================
# bench_iv_widen_ab.sh — same-window A/B for the IV-widening pass.
#
# COMPATIBILITY WRAPPER: forwards to scripts/perf_ab.py --preset iv_widen.
# The preset applies the identical kill switch the pre-consolidation script
# used (CCC_NO_IV_WIDEN=1 on the B side, default on the A side), verifies
# byte-identical stdout between the arms, and reports min-of-N over
# interleaved AB/BA rounds (the old script's blocked median-of-N protocol,
# upgraded to the unified noise-resistant screening protocol; both arms are
# compiled and timed back-to-back in the same window, so a frequency/thermal/
# scheduler shift hits both arms equally, and a speedup that miscompiles is
# still worthless and reported as a failure).
#
# WHY THIS EXISTS (preserved from the pre-consolidation script)
#
# This sandbox has no PMU (perf is not installed), so "run the timing harness"
# is the only performance signal available — and a single run of it is noise
# on a shared VM. The honest minimal protocol is the same-window interleaved
# A/B with output equality, which is what perf_ab.py implements.
#
# Usage:
#   scripts/bench_iv_widen_ab.sh                     # default kernel set
#   scripts/bench_iv_widen_ab.sh tls_seg_access      # one kernel
#   N=15 scripts/bench_iv_widen_ab.sh                # more rounds
#
# Environment:
#   LCCC_BIN  compiler under test (default target/fastbuild/lccc)
#   N         timed rounds per arm (default 7)       -> perf_ab --reps
#   KERNELS   space-separated kernel names under tests/benchmark/programs/
#   CPU       taskset CPU list to pin (default 0)    -> legacy-ignored, see note
# ============================================================================
set -euo pipefail

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
LCCC_BIN=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
N=${N:-7}
KERNELS=${KERNELS:-tls_seg_access sieve loop_patterns histogram nbody sqlite_varint arith_loop zlib_ng_adler32}
if [[ $# -gt 0 ]]; then KERNELS="$*"; fi

if [[ -n ${CPU:-} ]]; then
    # Legacy-ignored (perf_ab interleaves the A/B arms inside one timed window
    # instead of pinning separate blocks, which cancels the same scheduler /
    # frequency drifts the pinning used to guard against):
    echo "note: CPU=$CPU is accepted for compatibility but ignored: perf_ab interleaves the A/B arms back-to-back in the same window instead of pinning separate timed blocks" >&2
fi

[[ -x "$LCCC_BIN" ]] || { echo "error: $LCCC_BIN not found; build first" >&2; exit 2; }

only=""
for k in $KERNELS; do
    if [[ ! -f "$REPO/tests/benchmark/programs/$k.c" ]]; then
        echo "$k: missing (no tests/benchmark/programs/$k.c)" >&2
        continue
    fi
    only="${only:+$only,}$k"
done
[[ -n "$only" ]] || { echo "error: no benchmark sources matched the kernel list" >&2; exit 2; }

exec python3 "$REPO/scripts/perf_ab.py" \
    --preset iv_widen \
    --compiler-a "$LCCC_BIN" \
    --reps "$N" \
    --only "$only"
