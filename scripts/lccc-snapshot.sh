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
#   * The canonical deliverable /home/user/ms178-1.patch and independent root
#     mirror /home/user/ms178-1.latest.patch are refreshed on every call.
#   * /home/user/ms178-1.manifest pins base/head/size/SHA-256/apply verdict.
#   * The latest upstream/main base is enforced, and an invalid/empty patch is
#     a hard error rather than a misleading successful snapshot.
#   * A machine-readable ledger records what each save contains.
#
# Usage:  ./lccc-snapshot.sh "<slug>" "<one-line description>"
# ============================================================================
set -euo pipefail

REPO=${LCCC_REPO:-/home/user/lccc}
ART=${LCCC_ARTIFACTS:-/home/user/artifacts}
BASE_REF_FILE="$ART/.base_ref"
DELIVERABLE=/home/user/ms178-1.patch
ROOT_MIRROR=/home/user/ms178-1.latest.patch
MANIFEST=/home/user/ms178-1.manifest
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
if [[ -f "$BASE_REF_FILE" ]]; then
  BASE=$(cat "$BASE_REF_FILE")
else
  BASE=$(git rev-parse HEAD)
  printf '%s\n' "$BASE" > "$BASE_REF_FILE"
fi

# Refuse to manufacture a patch against a stale base. This fetch is cheap and
# turns the user's "latest main" requirement into an enforced invariant rather
# than a session convention. Set LCCC_SNAPSHOT_SKIP_FETCH=1 only for an
# explicitly offline recovery.
if [[ ${LCCC_SNAPSHOT_SKIP_FETCH:-0} != 1 ]]; then
  git fetch -q upstream main
fi
if git rev-parse --verify -q refs/remotes/upstream/main >/dev/null; then
  upstream=$(git rev-parse refs/remotes/upstream/main)
  if [[ $BASE != "$upstream" ]]; then
    echo "error: snapshot base $BASE is not latest upstream/main $upstream; rebase first" >&2
    exit 1
  fi
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
cp -f "$DELIVERABLE" "$ROOT_MIRROR"
cp -f "$DELIVERABLE" "$ART/ms178-1.patch"
cp -f "$DELIVERABLE" "$ART/ms178-1.${tag}.patch"

# ---- 2. per-commit series (reviewable history) ------------------------------
rm -rf "$ART/series"; mkdir -p "$ART/series"
git format-patch -q -o "$ART/series" "$BASE..HEAD" >/dev/null 2>&1 || true

# ---- 3. full source tarball (survives even if patch application breaks) -----
tar --exclude=.git --exclude=target --exclude=node_modules \
    --exclude=.godbolt-cache --exclude='*/__pycache__' \
    -czf "$ART/lccc-src.tar.gz.tmp.$$" -C "$(dirname "$REPO")" "$(basename "$REPO")" 2>/dev/null || true
mv -f "$ART/lccc-src.tar.gz.tmp.$$" "$ART/lccc-src.tar.gz"

# ---- 4. git bundle (full history, restores worktree + commits) --------------
git bundle create "$ART/lccc.bundle.tmp.$$" --all >/dev/null 2>&1 || true
mv -f "$ART/lccc.bundle.tmp.$$" "$ART/lccc.bundle" 2>/dev/null || true

# ---- 5. ledger --------------------------------------------------------------
files=$(git diff --stat "$BASE" HEAD | tail -1 | sed 's/^ *//')
if [[ ! -f "$LEDGER" ]]; then
  {
    echo "# LCCC Snapshot Ledger"
    echo
    echo "Base (upstream): \`$BASE\`"
    echo
    echo "| # | UTC | Tag | Description | Cumulative diffstat |"
    echo "|---|-----|-----|-------------|---------------------|"
  } > "$LEDGER"
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

# A snapshot that cannot restore is worse than no snapshot because it creates
# false confidence. Fail hard after preserving the evidence for diagnosis.
if [[ $verdict != APPLIES-CLEAN ]]; then
  echo "error: refusing invalid snapshot: $verdict ($bytes bytes)" >&2
  exit 1
fi

sha256=$(sha256sum "$DELIVERABLE" | awk '{print $1}')
{
  printf 'schema=1\n'
  printf 'utc=%s\n' "$stamp"
  printf 'tag=%s\n' "$tag"
  printf 'base=%s\n' "$BASE"
  printf 'head=%s\n' "$HEAD_SHA"
  printf 'bytes=%s\n' "$bytes"
  printf 'sha256=%s\n' "$sha256"
  printf 'verdict=%s\n' "$verdict"
} | atomic_write "$MANIFEST"
cp -f "$MANIFEST" "$ART/ms178-1.manifest"

# Arena persists at most roughly 10k files. Generated kernel/fuzz trees are
# reproducible and must not crowd the canonical patch out of the snapshot.
# This warning is intentionally visible on every save above the safety margin.
persist_files=$(find /home/user -xdev -type f \
  ! -path '/home/user/lccc/target/*' ! -path '/home/user/.cache/*' 2>/dev/null | wc -l)
if (( persist_files > 8000 )); then
  echo "warning: workspace has $persist_files persistable files (8k safety limit); prune generated kernel/fuzz outputs" >&2
fi

# Flush only the snapshot products. A process-wide `sync` also waits for every
# dirty incremental-build object under target/; on this VM that turned a 20 MB
# snapshot into a 20-minute stall after each build. fsyncing the deliverables
# and their directory provides the durability this script needs without making
# unrelated compiler caches part of the critical path.
fsync_path() {
  python3 - "$1" <<'PY' 2>/dev/null || true
import os, sys
fd = os.open(sys.argv[1], os.O_RDONLY)
try:
    os.fsync(fd)
finally:
    os.close(fd)
PY
}
for saved in "$DELIVERABLE" "$ROOT_MIRROR" "$MANIFEST" \
             "$ART/ms178-1.patch" "$ART/ms178-1.manifest" \
             "$ART/ms178-1.${tag}.patch" "$ART/lccc-src.tar.gz" \
             "$ART/lccc.bundle" "$LEDGER" "$seq_file"; do
  [[ -e "$saved" ]] && fsync_path "$saved"
done
fsync_path "$ART"
fsync_path "$(dirname "$DELIVERABLE")"

echo "SNAPSHOT $tag"
echo "  base       : $BASE"
echo "  head       : $HEAD_SHA"
echo "  deliverable: $DELIVERABLE ($bytes bytes) [$verdict]"
echo "  sha256     : $sha256"
echo "  mirror     : $ROOT_MIRROR"
echo "  manifest   : $MANIFEST"
echo "  files      : $persist_files persistable"
echo "  artifacts  : $ART"
