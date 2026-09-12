#!/usr/bin/env bash
# Snapshot the current P0-A (ms178-1) working tree into the user-facing
# workspace. Standing order: re-run after every validated fix. Idempotent.
set -euo pipefail
REPO=${LCCC_REPO:-$HOME/lccc}
OUTDIR=${MS178_OUTDIR:-$HOME/patches}
mkdir -p "$OUTDIR"
cd "$REPO"
git add -A -N
TS=$(date +%Y%m%d-%H%M%S)
OUT="$OUTDIR/ms178-1.$TS.patch"
# GLA work touches these paths; include them explicitly so the snapshot is
# self-contained even when other untracked files exist in the tree.
git diff HEAD -- \
  src/backend/location_alloc.rs \
  src/backend/mod.rs \
  src/backend/split_ranges.rs \
  src/driver/pipeline.rs \
  scripts/snapshot_ms178.sh \
  scripts/ci_local.sh \
  tests/regression/check_gla_remat_policy.sh \
  engineering/FOLLOWUP-2026-09-11-global-location-allocation-phase1.md \
  engineering/DECISIONS.md \
  artifacts/repros/crash_gen_slot_stress_seed20260930_-O2.c \
  > "$OUT"
git reset -q
ln -sf "ms178-1.$TS.patch" "$OUTDIR/ms178-1.patch"
cp -f "$OUT" "$HOME/ms178-1.patch"
echo "snapshot: $OUT ($(wc -l < "$OUT") lines); also $HOME/ms178-1.patch"
