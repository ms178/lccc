#!/usr/bin/env bash
# Paired A/B performance sweep for two LCCC binaries over the whole
# benchmark corpus at -O1/-O2/-O3/-Os.
#
#   scripts/ab_pair_sweep.sh MINE LCCC REF LCCC OUTDIR [EXTRA ...]
#
# Example (x86-64):
#   scripts/ab_pair_sweep.sh target/fastbuild/lccc \
#       /home/user/refbuilds/refA/target/fastbuild/lccc \
#       /home/user/evidence/ab_x64
# Example (i686 cross binaries):
#   scripts/ab_pair_sweep.sh target/fastbuild/lccc-i686 \
#       /home/user/refbuilds/refA/target/fastbuild/lccc-i686 \
#       /home/user/evidence/ab_i686
#
# Every level produces OUTDIR/ab_<tag>.{json,md} via run_benchmarks.py
# (paired rounds, randomized order, median + paired bootstrap CI,
# correctness oracle). EXTRA arguments are forwarded to the runner
# (e.g. --reps 25 --seed 7).  Requires the ref build to exist; see
# scripts/arena_session_restore.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

MINE=${1:?usage: ab_pair_sweep.sh MINE REF OUTDIR [EXTRA...]}
REF=${2:?missing REF lccc path}
OUTDIR=${3:?missing OUTDIR}
shift 3 || true
[[ -x $MINE ]] || { echo "missing MINE binary: $MINE" >&2; exit 2; }
[[ -x $REF ]]  || { echo "missing REF binary: $REF"  >&2; exit 2; }
mkdir -p "$OUTDIR"

REPS=${AB_REPS:-15}
WARM=${AB_WARM:-2}
SEED=${AB_SEED:-4}

for opt in -O1 -O2 -O3 -Os; do
  tag=$(echo "$opt" | tr -d -)
  echo "=== A/B SWEEP $opt start $(date +%T) ==="
  python3 tests/benchmark/run_benchmarks.py \
    --compilers lccc,ccc --lccc "$MINE" --ccc "$REF" \
    --opt="$opt" --reps "$REPS" --warmup "$WARM" --no-pmu-probe \
    --seed "$SEED" --strict \
    --json "$OUTDIR/ab_$tag.json" --markdown "$OUTDIR/ab_$tag.md" "$@"
  echo "=== A/B SWEEP $opt done $(date +%T) ==="
done
echo AB-PAIR-SWEEP-DONE
