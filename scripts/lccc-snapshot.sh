#!/usr/bin/env bash
# ============================================================================
# LCCC session snapshot ("Kontinuität") — harness-wipe-resistant autosave.
#
# The execution harness is known to wipe all data outside the persisted
# workspace root (/home/user), and to die at arbitrary points.  Therefore:
#
#   * Every validated improvement is committed in the git worktree AND
#     immediately materialised as a flat patch + full source tarball under
#     /home/user/artifacts (persisted).
#   * Writes are atomic (write to .tmp, fsync, rename) so a wipe/kill in the
#     middle of a save can never leave a truncated deliverable.
#   * The canonical deliverable /home/user/ms178-1.patch is refreshed on every
#     call, so at any instant the workspace holds the complete session work.
#   * A machine-readable ledger records what each save contains.
#
# Usage:  ./lccc-snapshot.sh "<slug>" "<one-line description>"
# ============================================================================
set -euo pipefail

REPO=${LCCC_REPO:-/home/user/lccc}
ART=${LCCC_ARTIFACTS:-/home/user/artifacts}
BASE_REF_FILE="$ART/.base_ref"
# Deliverable location is overridable so the script works in any workspace
# (the Arena harness persists /home/user; other environments point it at their
# own durable directory, e.g. a CI artifact mount).
DELIVERABLE=${LCCC_DELIVERABLE:-/home/user/ms178-1.patch}
LEDGER="$ART/SNAPSHOT_LEDGER.md"

slug=${1:-snapshot}
desc=${2:-"session snapshot"}
stamp=$(date -u +%Y%m%dT%H%M%SZ)
seq_file="$ART/.seq"

mkdir -p "$ART"
[[ -f "$seq_file" ]] || echo 0 > "$seq_file"
seq=$(( $(cat "$seq_file") + 1 ))
printf '%s\n' "$seq" > "$seq_file"
tag=$(printf 'S%02d-%s' "$seq" "$slug")

cd "$REPO"

# ---- base ref: the upstream commit this session is rebased on ---------------
# The deliverable must contain exactly this session's work, so the base is the
# UPSTREAM commit the session started from, never the session's own commits.
# On first use that is origin/main (the clone point / rebase target); a
# recorded base survives between sessions so a later snapshot keeps the full
# session history even after the branch has diverged further.
BASE=""
if [[ -f "$BASE_REF_FILE" ]]; then
  BASE=$(cat "$BASE_REF_FILE")
  if ! git cat-file -e "$BASE^{commit}" 2>/dev/null; then
    echo "lccc-snapshot: recorded base $BASE no longer exists; resetting to origin/main" >&2
    BASE=""
  fi
fi
if [[ -z "$BASE" ]]; then
  BASE=$(git rev-parse --verify --quiet origin/main || git rev-parse HEAD)
  printf '%s\n' "$BASE" > "$BASE_REF_FILE"
fi

# ---- commit any pending work ------------------------------------------------
if ! git diff --quiet || ! git diff --cached --quiet || \
   [[ -n "$(git ls-files --others --exclude-standard)" ]]; then
  git add -A
  git -c user.name='LCCC Agent' -c user.email='agent@lccc.local' \
      commit -q -m "$slug: $desc" || true
fi

HEAD_SHA=$(git rev-parse HEAD)

# ---- atomic writer ----------------------------------------------------------
atomic_write() {  # atomic_write <dest> ; content on stdin
  local dest=$1 tmp
  tmp="$dest.tmp.$$"
  cat > "$tmp"
  sync -f "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$dest"
}

# ---- 1. canonical deliverable: single squashed patch base..HEAD -------------
if [[ "$HEAD_SHA" != "$BASE" ]]; then
  git diff --binary --no-color "$BASE" HEAD | atomic_write "$DELIVERABLE"
else
  : | atomic_write "$DELIVERABLE"
fi
cp -f "$DELIVERABLE" "$ART/ms178-1.patch"
cp -f "$DELIVERABLE" "$ART/ms178-1.${tag}.patch"

# ---- 2. per-commit series (reviewable history) ------------------------------
rm -rf "$ART/series"; mkdir -p "$ART/series"
git format-patch -q -o "$ART/series" "$BASE..HEAD" >/dev/null 2>&1 || true

# ---- 3. full source tarball (survives even if patch application breaks) -----
tar --exclude=.git --exclude=target --exclude=node_modules \
    -czf "$ART/lccc-src.tar.gz.tmp.$$" -C "$(dirname "$REPO")" "$(basename "$REPO")" 2>/dev/null || true
mv -f "$ART/lccc-src.tar.gz.tmp.$$" "$ART/lccc-src.tar.gz"

# ---- 4. git bundle (full history, restores worktree + commits) --------------
git bundle create "$ART/lccc.bundle.tmp.$$" --all >/dev/null 2>&1 || true
mv -f "$ART/lccc.bundle.tmp.$$" "$ART/lccc.bundle" 2>/dev/null || true

# ---- 5. ledger --------------------------------------------------------------
files=$(git diff --stat "$BASE" HEAD | tail -1 | sed 's/^ *//')
if [[ ! -f "$LEDGER" ]]; then
  echo "# LCCC Snapshot Ledger" > "$LEDGER"
fi
# A merged session starts from a new upstream base while retaining historical
# artifacts. Start a fresh ledger section instead of silently appending rows
# under the previous base's heading.
if ! grep -Fq "Base (upstream): \`$BASE\`" "$LEDGER"; then
  {
    echo
    echo "## Base (upstream): \`$BASE\`"
    echo
    echo "| # | UTC | Tag | Description | Cumulative diffstat |"
    echo "|---|-----|-----|-------------|---------------------|"
  } >> "$LEDGER"
fi
printf '| %d | %s | `%s` | %s | %s |\n' "$seq" "$stamp" "$tag" "$desc" "${files:-none}" >> "$LEDGER"

# ---- 6. verification: the saved patch must be non-trivial and applicable ----
bytes=$(wc -c < "$DELIVERABLE")
verdict="EMPTY"
if [[ "$bytes" -gt 0 ]]; then
  tmpd=$(mktemp -d)
  if git -C "$REPO" worktree add -q --detach "$tmpd" "$BASE" 2>/dev/null; then
    if git -C "$tmpd" apply --check "$DELIVERABLE" 2>/dev/null; then
      verdict="APPLIES-CLEAN"
    else
      verdict="APPLY-FAILED"
    fi
    git -C "$REPO" worktree remove --force "$tmpd" 2>/dev/null || true
  else
    verdict="UNVERIFIED"
  fi
  rm -rf "$tmpd"
fi

sync 2>/dev/null || true

echo "SNAPSHOT $tag"
echo "  base       : $BASE"
echo "  head       : $HEAD_SHA"
echo "  deliverable: $DELIVERABLE ($bytes bytes) [$verdict]"
echo "  artifacts  : $ART"
