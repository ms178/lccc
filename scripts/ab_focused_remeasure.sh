#!/usr/bin/env bash
# Focused multi-seed paired re-measurement for benchmarks whose 15-rep
# sweep CI excluded 1.0. Runs run_benchmarks.py repeatedly with distinct
# seeds (more reps each), then reports the median-of-seeds ratio so a
# spurious single-seed CI cannot be mistaken for a real regression.
#
#   scripts/ab_focused_remeasure.sh MINE REF OPT "bench1,bench2" [REPS] [SEEDS...]
set -uo pipefail
cd "$(dirname "$0")/.."
MINE=${1:?}; REF=${2:?}; OPT=${3:?}; ONLY=${4:?}; REPS=${5:-31}
shift 5 || true
SEEDS=("$@"); [ ${#SEEDS[@]} -gt 0 ] || SEEDS=(3 11 27)
OUTDIR=/tmp/focused_$(echo "$OPT" | tr -d -)
mkdir -p "$OUTDIR"
for seed in "${SEEDS[@]}"; do
  echo "=== focused $OPT $ONLY seed=$seed reps=$REPS ==="
  python3 tests/benchmark/run_benchmarks.py \
    --compilers lccc,ccc --lccc "$MINE" --ccc "$REF" \
    --opt="$OPT" --only "$ONLY" --reps "$REPS" --warmup 4 \
    --no-pmu-probe --seed "$seed" --strict \
    --json "$OUTDIR/s$seed.json" --markdown "$OUTDIR/s$seed.md" >/dev/null 2>&1 \
    && grep -E "^\| " "$OUTDIR/s$seed.md" | tail -n +2
done
python3 - "$OUTDIR" <<'PY'
import json, glob, math, sys, statistics
out=sys.argv[1]
agg={}
for f in sorted(glob.glob(out+'/s*.json')):
    d=json.load(open(f))
    for b in d['benchmarks']:
        n=b.get('name') or b.get('benchmark') or b.get('id')
        r=(b.get('ratios_from_lccc') or {}).get('ccc',{}).get('median')
        if r: agg.setdefault(n,[]).append(r)
print('\n== median-of-seeds paired ratio (>1 = slower) ==')
for n,xs in sorted(agg.items()):
    print(f"{n:24s} seeds={xs} median={statistics.median(xs):.4f} "
          f"min={min(xs):.4f} max={max(xs):.4f}")
PY
