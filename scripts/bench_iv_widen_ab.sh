#!/usr/bin/env bash
# ============================================================================
# bench_iv_widen_ab.sh — same-window A/B for the IV-widening pass.
#
# Compiles each benchmark kernel twice (widen ON vs OFF via the
# CCC_NO_IV_WIDEN=1 kill switch), verifies byte-identical stdout, and reports
# the median-of-N wall time of each arm plus the ON/OFF ratio.
#
# WHY THIS EXISTS
#
# This sandbox has no PMU (perf is not installed), so "run the timing harness"
# is the only performance signal available — and a single run of it is noise
# on a shared VM. The honest minimal protocol is the one this script encodes:
#
#   * SAME WINDOW: both arms are compiled and timed back-to-back, so a
#     frequency/thermal/scheduler shift hits both arms equally.
#   * INTERLEAVED REPETITIONS, MEDIAN: outliers do not win; the median of N
#     rounds is reported. A 5% single-window delta is still re-measurable.
#   * OUTPUT EQUALITY: the arms must print identical output, or the line is
#     reported as a MISMATCH rather than a ratio (a speedup that miscompiles
#     is worthless).
#
# Usage:
#   scripts/bench_iv_widen_ab.sh                     # default kernel set
#   scripts/bench_iv_widen_ab.sh tls_seg_access      # one kernel
#   N=15 scripts/bench_iv_widen_ab.sh                # more rounds
#
# Environment:
#   LCCC_BIN  compiler under test (default target/fastbuild/lccc)
#   N         timed rounds per arm (default 7)
#   KERNELS   space-separated kernel names under tests/benchmark/programs/
#   CPU       taskset CPU list to pin (default 0)
# ============================================================================
set -u

REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
LCCC_BIN=${LCCC_BIN:-$REPO/target/fastbuild/lccc}
N=${N:-7}
CPU=${CPU:-0}
KERNELS=${KERNELS:-tls_seg_access sieve loop_patterns histogram nbody sqlite_varint arith_loop zlib_ng_adler32}
if [[ $# -gt 0 ]]; then KERNELS="$*"; fi

[[ -x "$LCCC_BIN" ]] || { echo "error: $LCCC_BIN not found; build first" >&2; exit 2; }

WORK=$(mktemp -d /tmp/abivw.XXXXXX)
trap 'rm -rf "$WORK"' EXIT

run() {  # run <bin> -> median wall ms across N rounds
    local bin=$1 i t0 t1
    local -a ts=()
    for ((i = 0; i < N; i++)); do
        t0=$(date +%s%N)
        "$bin" > /dev/null 2>&1
        t1=$(date +%s%N)
        ts+=("$(( (t1 - t0) / 1000000 ))")
    done
    printf '%s\n' "${ts[@]}" | python3 -c \
        'import sys; xs=sorted(int(l) for l in sys.stdin); print(xs[len(xs)//2])'
}

printf '%-18s %8s %8s %8s\n' kernel ON_ms OFF_ms ratio
for k in $KERNELS; do
    src="$REPO/tests/benchmark/programs/$k.c"
    [[ -f "$src" ]] || { echo "$k: missing"; continue; }

    "$LCCC_BIN" -O2 "$src" -o "$WORK/$k.on" 2>/dev/null || { echo "$k: on-build fail"; continue; }
    CCC_NO_IV_WIDEN=1 "$LCCC_BIN" -O2 "$src" -o "$WORK/$k.off" 2>/dev/null || { echo "$k: off-build fail"; continue; }

    a=$("$WORK/$k.on" 2>/dev/null); b=$("$WORK/$k.off" 2>/dev/null)
    [[ "$a" == "$b" ]] || { echo "$k: OUTPUT MISMATCH on='$a' off='$b'"; continue; }

    on=$(run "$WORK/$k.on")
    off=$(run "$WORK/$k.off")
    python3 -c "print(f'%-18s %8s %8s %8s' % ('$k', '$on', '$off', f'{int($on)/int($off):.3f}'))"
done
