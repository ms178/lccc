#!/usr/bin/env bash
# Snapshot the current ms178-1 follow-up working tree into the user-facing
# workspace. Standing order: re-run after every validated fix. Idempotent.
# The patch is the full delta of this branch against its merge-base with
# origin/main, so it stays correct across rebases without a path manifest.
set -euo pipefail
REPO=${LCCC_REPO:-$HOME/lccc}
OUTDIR=${MS178_OUTDIR:-$HOME/patches}
mkdir -p "$OUTDIR"
cd "$REPO"
# Mark every untracked file intent-to-add so the diff includes new tests.
git add -A -N
BASE=$(git merge-base HEAD origin/main)
TS=$(date +%Y%m%d-%H%M%S)
OUT="$OUTDIR/ms178-1.$TS.patch"
git diff "$BASE" -- > "$OUT"
git reset -q
ln -sf "ms178-1.$TS.patch" "$OUTDIR/ms178-1.patch"
cp -f "$OUT" "$HOME/ms178-1.patch"
echo "base: $BASE"
echo "snapshot: $OUT ($(wc -l < "$OUT") lines); also $HOME/ms178-1.patch"
