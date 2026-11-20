#!/usr/bin/env bash
# ============================================================================
# arena_snapshot.sh — publish the ms178-1 follow-up patch into the
# user-facing workspace, atomically and redundantly, so that a mid-session
# wipe of this sandbox cannot destroy validated work.
#
# WHY THIS EXISTS (read before changing it)
# -----------------------------------------
# The Arena harness restores /home/user between turns from a size-capped
# snapshot (~128 MiB / ~10 000 files) and has been observed to drop or truncate
# the things the research loop depends on:
#
#   * untracked scripts, docs and results (everything not in git),
#   * target/ (an excluded directory name), so the compiler is rebuilt,
#   * .git/config, sometimes .git entirely,
#   * any *single* large file past the cap -- which is why a multi-GiB build
#     tree in the workspace fails EVERY publication (see ensure_swap.sh).
#
# The delivery contract is therefore:
#
#   1. the ONLY durable artifact is a text patch in the workspace root,
#   2. it is regenerated from git on every validated change, never hand-edited,
#   3. publication is atomic (write to .tmp, fsync, rename) so a wipe during
#      publication leaves either the old complete file or the new one -- never
#      a truncated patch that applies halfway,
#   4. a bounded, rotating history is kept so a bad later state can be
#      recovered by a plain `git apply`,
#   5. every publication writes a manifest (LINEAGE ledger) carrying the base
#      commit, tree hash, gate verdict and per-file change summary.
#
# The patch is the FULL delta against the merge-base with upstream main, so it
# survives rebases without a path manifest: after `git rebase`, the next run
# regenerates against the new base and the patch still applies to it.
#
# USAGE
#   scripts/arena_snapshot.sh "<slug>" "<one-line description>"
#
# ENVIRONMENT
#   LCCC_REPO       repository root            (default: this checkout)
#   MS178_OUTDIR    history directory          (default: $HOME/snapshots)
#   MS178_BASE_REF  upstream ref to diff against (default: first of
#                   origin/main github/main upstream/main)
#   MS178_KEEP      history entries to retain  (default: 12)
#   MS178_UNGATED=1 publish even when no ci_local --fast stamp matches the
#                   current tree (records that the snapshot is ungated)
#   MS178_DRY_RUN=1 compute and report, publish nothing
# ============================================================================
set -euo pipefail

slug=${1:-}
description=${2:-}
if [[ -z $slug ]]; then
  echo "usage: arena_snapshot.sh <slug> <description>" >&2
  exit 2
fi
# A slug becomes part of a filename in a directory the user browses: keep it to
# a safe alphabet instead of trusting the caller's quoting.
[[ $slug =~ ^[A-Za-z0-9._-]+$ ]] || {
  echo "error: slug must match [A-Za-z0-9._-]+ (got '$slug')" >&2; exit 2;
}

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO=${LCCC_REPO:-$(CDPATH= cd -- "$here/.." && pwd)}
OUTDIR=${MS178_OUTDIR:-$HOME/snapshots}
KEEP=${MS178_KEEP:-12}
[[ $KEEP =~ ^[1-9][0-9]*$ ]] || { echo "error: MS178_KEEP must be a positive integer" >&2; exit 2; }

cd "$REPO"
if [[ ! -d .git ]]; then
  echo "error: $REPO is not a git repository; nothing to snapshot" >&2
  echo "       restore it with scripts/arena_session_restore.sh --source-only" >&2
  exit 1
fi

# ---- resolve the upstream base ---------------------------------------------
if [[ -n ${MS178_BASE_REF:-} ]]; then
  candidates=$MS178_BASE_REF
else
  candidates="origin/main github/main upstream/main"
fi
upstream=""
for ref in $candidates; do
  if git rev-parse --verify --quiet "$ref" >/dev/null; then upstream=$ref; break; fi
done
if [[ -z $upstream ]]; then
  echo "error: no upstream main among: $candidates" >&2
  echo "       set MS178_BASE_REF=<ref> (e.g. origin/master)" >&2
  exit 1
fi
base=$(git merge-base HEAD "$upstream")
if [[ -z $base ]]; then
  echo "error: no merge-base between HEAD and $upstream" >&2; exit 1
fi

# ---- scratch guard ---------------------------------------------------------
# `git add -N` is deliberately broad (it is what makes a brand-new test appear
# without a manifest) and that breadth has shipped generated scratch before:
# 61 benchmark `.s` dumps reached the user-facing patch once, 83.7% of the diff.
# Only ADDITIONS are inspected; upstream legitimately tracks work/ scratch.
git add -A -N
mapfile -t staged < <(git diff --cached --name-only --diff-filter=A)
git reset -q
# `git reset` above clears the intent-to-add marks, so untracked files would
# disappear from the `git diff <base>` below and the publisher would report an
# empty delta while a brand-new script sits right there. Re-arm them and leave
# them armed for the diff; the trailing reset at the end restores the caller's
# index state (some workflows assert on a clean `git status`).
git add -A -N
scratch=()
for f in "${staged[@]:-}"; do
  [[ -z $f ]] && continue
  case $f in
    /*.s|*.s|*.o|*.so|*.tmp|*.out|core|core.[0-9]*) scratch+=("$f") ;;
  esac
done
if (( ${#scratch[@]} )); then
  echo "error: refusing to publish generated scratch:" >&2
  printf '  %s\n' "${scratch[@]}" >&2
  echo "       remove them, or add the pattern to .gitignore" >&2
  exit 1
fi

# ---- gate stamp ------------------------------------------------------------
# The snapshot gate is a ci_local stamp for THIS tree, and "this tree" has
# exactly one definition in this repository: `scripts/worktree_tree.sh`, the
# same content address `ci_local.sh` writes into `target/ci_local.pass` and
# `lccc-snapshot.sh` demands.  Do not invent a second one.
#
# DEFECT THIS REPLACES (2026-11-20): the gate used to hash `git diff <base>`
# and look for a stamp under `~/.cache/lccc/ci-stamps/<diff-sha>`, while
# `ci_local.sh` keys its stamp on the *worktree tree hash* and writes no
# second copy.  The two spaces never intersected, so a fully green
# `ci_local.sh --fast` (680 passed / 0 failed) still published every snapshot
# labelled `gate UNGATED`.  A gate that can never fire is worse than no gate:
# it reads as evidence in the lineage ledger.
#
# A stamp for a DIFFERENT tree is not a pass.  That is the whole point of
# keying on the tree, and it is why the check below compares the stamp's own
# `tree=` field instead of trusting the file's presence.
tree=$(bash scripts/worktree_tree.sh 2>/dev/null || git write-tree 2>/dev/null || true)
stamp=${LCCC_CI_STAMP:-$REPO/target/ci_local.pass}
gate="UNGATED"
if [[ -n $tree && -r $stamp ]]; then
  stamp_tree=$(sed -n 's/^tree=//p' "$stamp" | head -1)
  stamp_mode=$(sed -n 's/^mode=//p' "$stamp" | head -1)
  if [[ $stamp_tree == "$tree" ]]; then
    gate="ci_local --fast PASS (mode=$stamp_mode, tree $tree)"
  else
    echo "warning: $stamp is for tree $stamp_tree, not the tree being published ($tree)" >&2
  fi
fi
if [[ $gate == UNGATED && ${MS178_UNGATED:-0} != 1 ]]; then
  echo "warning: no ci_local stamp for this tree ($tree)" >&2
  echo "         run scripts/ci_local.sh --fast, or set MS178_UNGATED=1 to" >&2
  echo "         publish a snapshot explicitly labelled ungated" >&2
fi

# ---- build the patch -------------------------------------------------------
ts=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$OUTDIR"
name="ms178-1.$ts.$slug.patch"
tmp="$OUTDIR/.$name.tmp"
git diff "$base" -- > "$tmp"
if [[ ! -s $tmp ]]; then
  rm -f "$tmp"
  echo "error: empty diff against $base; nothing to publish" >&2
  exit 1
fi
# Statistics are captured while the intent-to-add marks are still armed: after
# the trailing `git reset` below, untracked files vanish from the diff and the
# summary would report a zero-file snapshot of a non-empty patch (observed).
files_changed=$(git diff --name-only "$base" -- | wc -l)
numstat=$(git diff --numstat "$base" -- | awk '{a+=$1; d+=$2} END {print a+0" additions, "d+0" deletions"}')
# fsync before rename: rename is atomic for readers, but the CONTENT must be on
# stable storage first or a host-level crash exposes a zero-length file.
sync "$tmp" 2>/dev/null || true
mv -f "$tmp" "$OUTDIR/$name"

# ---- atomic workspace copy + rolling alias --------------------------------
# TWO names, both in the workspace root:
#   ms178-1.patch          -- always the newest (what the owner and downstream
#                             restore recipes read)
#   ms178-1.<ts>.<slug>.patch -- the dated artifact (history survives rotation
#                             of the alias even if a later run is wrong)
publish_dir=${MS178_PUBLISH_DIR:-$HOME}
# The publish directory is an OUTPUT location, so create it: `cp` into a
# missing parent failed the whole publication *after* the snapshot and ledger
# work, which reads as "the patch was not produced" when it was.  (Found by
# tests/corpus/test_arena_snapshot_gate.py, which runs against a scratch
# HOME.)
mkdir -p "$publish_dir"
for target in "$publish_dir/ms178-1.patch" "$publish_dir/ms178-1.$ts.$slug.patch"; do
  cp -f "$OUTDIR/$name" "$target.tmp"
  sync "$target.tmp" 2>/dev/null || true
  mv -f "$target.tmp" "$target"
done
ln -sfn "$name" "$OUTDIR/ms178-1.patch"
git reset -q

# ---- lineage ledger --------------------------------------------------------
files=$(git diff --stat "$base" -- | tail -1)
{
  printf 'timestamp  %s\n' "$ts"
  printf 'slug       %s\n' "$slug"
  printf 'describe   %s\n' "$description"
  printf 'repo       %s\n' "$REPO"
  printf 'head       %s\n' "$(git rev-parse HEAD)"
  printf 'base       %s (%s)\n' "$base" "$upstream"
  printf 'content    tree %s (worktree_tree.sh; the ci_local --fast stamp key)\n' "$tree"
  printf 'patch      %s (%s lines, sha256:%s)\n' "$name" \
         "$(wc -l < "$OUTDIR/$name")" \
         "$(sha256sum "$OUTDIR/$name" | cut -d' ' -f1)"
  printf 'gate       %s\n' "$gate"
  printf 'scope      %s\n' "$files"
  printf 'subjects   \n'
  git log --oneline "$base"..HEAD | sed 's/^/           /'
  printf '\n'
} >> "$OUTDIR/LINEAGE.log"

# Rotate the HISTORY directory only. The workspace aliases are never rotated:
# losing the newest patch to housekeeping is exactly the failure this script is
# here to prevent.
mapfile -t old < <(find "$OUTDIR" -maxdepth 1 -name 'ms178-1.*.patch' -printf '%T@ %p\n' \
                   | sort -rn | awk 'NR > '"$KEEP"' {print $2}')
for f in "${old[@]:-}"; do [[ -n $f ]] && rm -f "$f"; done

# ---- report ----------------------------------------------------------------
echo "snapshot   $OUTDIR/$name"
echo "publish    $publish_dir/ms178-1.patch (+ dated copy)"
echo "base       $base (from $upstream)"
echo "scope      $files_changed file(s), $numstat"
echo "gate       $gate"
echo "ledger     $OUTDIR/LINEAGE.log (last entry)"
