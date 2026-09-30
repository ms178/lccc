#!/usr/bin/env bash
# Snapshot the current ms178-1 follow-up working tree into the user-facing
# workspace. Standing order: re-run after every validated fix. Idempotent.
# The patch is the full delta of this branch against its merge-base with
# upstream main, so it stays correct across rebases without a path manifest.
#
# The upstream ref name is not portable: clones disagree on it (origin/main vs
# github/main vs upstream/main). Resolve the first that exists, and allow an
# explicit override via MS178_BASE_REF. An unresolvable base is a hard error --
# silently diffing against the wrong ref would ship a patch that does not apply.
set -euo pipefail
REPO=${LCCC_REPO:-$HOME/lccc}
OUTDIR=${MS178_OUTDIR:-$HOME/patches}
mkdir -p "$OUTDIR"
cd "$REPO"
if [ -n "${MS178_BASE_REF:-}" ]; then
    CANDIDATES=$MS178_BASE_REF
else
    CANDIDATES="origin/main github/main upstream/main"
fi
UPSTREAM=""
for ref in $CANDIDATES; do
    if git rev-parse --verify --quiet "$ref" >/dev/null; then
        UPSTREAM=$ref
        break
    fi
done
if [ -z "$UPSTREAM" ]; then
    echo "error: no upstream main found (tried: $CANDIDATES)." >&2
    echo "       set MS178_BASE_REF to the ref to diff against." >&2
    exit 1
fi
# Mark every untracked file intent-to-add so the diff includes new tests.
git add -A -N

# Refuse to ship generated scratch. `git add -A -N` is deliberately broad --
# it is what makes a brand-new regression test show up without a manifest --
# and on 2026-09-30 that breadth put 61 benchmark `.s` dumps in the repository
# root into a deliverable: 21,483 lines, 83.7% of the diff, none of it reviewable.
# .gitignore now covers `/*.s`, but a silent, total absence of these artifacts
# is exactly the signal worth asserting: if one reaches a patch, either the
# ignore rule regressed or a new generator is writing somewhere unignored.
#
# Only ADDITIONS are inspected. `work/` and friends are scratch upstream
# already tracks, so flagging them would make the guard fire on a clean tree
# and train everyone to reach for `--no-verify`.
STAGED=$(git diff --cached --name-only --diff-filter=A)
SCRATCH=$(printf '%s\n' "$STAGED" | grep -E '^[^/]*\.s$|\.o$|\.so$|\.tmp$|\.out$|^core[.0-9]*$' || true)
if [ -n "$SCRATCH" ]; then
    git reset -q
    echo "error: refusing to snapshot generated scratch:" >&2
    printf '  %s\n' $SCRATCH >&2
    echo "       Delete them, or add the pattern to .gitignore." >&2
    exit 1
fi

BASE=$(git merge-base HEAD "$UPSTREAM")
TS=$(date +%Y%m%d-%H%M%S)
OUT="$OUTDIR/ms178-1.$TS.patch"
git diff "$BASE" -- > "$OUT"
git reset -q
ln -sf "ms178-1.$TS.patch" "$OUTDIR/ms178-1.patch"
cp -f "$OUT" "$HOME/ms178-1.patch"
echo "base: $BASE (from $UPSTREAM)"
echo "snapshot: $OUT ($(wc -l < "$OUT") lines); also $HOME/ms178-1.patch"
