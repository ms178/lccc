#!/usr/bin/env bash
# ============================================================================
# LCCC session snapshot ("Kontinuität") — harness-wipe-resistant autosave.
#
# The execution harness may wipe storage or terminate this script at arbitrary
# points.  A snapshot therefore has two invariants:
#
#   * source is committed before it is published; and
#   * every published file is built and verified off-path, then installed with
#     an atomic rename.  A kill can preserve an older complete snapshot or
#     leave a disposable .tmp file, but cannot publish a truncated patch,
#     archive, bundle, or ledger.
#
# The canonical deliverable /home/user/ms178-1.patch is refreshed on every
# successful call.  The snapshot also records a reviewable format-patch series,
# source tarball, git bundle, and ledger under /home/user/artifacts.
#
# Usage:  ./lccc-snapshot.sh "<slug>" "<one-line description>"
# Env:    LCCC_REPO, LCCC_ARTIFACTS, LCCC_DELIVERABLE
#         LCCC_BASE_REF — explicit upstream base for a rebase/new session.
#                         When set, it atomically supersedes artifacts/.base_ref
#                         after ancestry validation.
# ============================================================================
set -euo pipefail

REPO=${LCCC_REPO:-/home/user/lccc}
ART=${LCCC_ARTIFACTS:-/home/user/artifacts}
BASE_REF_FILE="$ART/.base_ref"
DELIVERABLE=${LCCC_DELIVERABLE:-/home/user/ms178-1.patch}
LEDGER="$ART/SNAPSHOT_LEDGER.md"

slug=${1:-snapshot}
desc=${2:-"session snapshot"}
stamp=$(date -u +%Y%m%dT%H%M%SZ)
seq_file="$ART/.seq"

if [[ ! $slug =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
  printf 'invalid snapshot slug %q (use letters, digits, dot, underscore, dash)\n' "$slug" >&2
  exit 2
fi
desc=${desc//$'\n'/ }
desc=${desc//$'\r'/ }

mkdir -p "$ART" "$(dirname "$DELIVERABLE")"
cd "$REPO"

atomic_write() {  # atomic_write <destination>; content arrives on stdin
  local dest=$1 tmp
  mkdir -p "$(dirname "$dest")"
  tmp=$(mktemp "${dest}.tmp.XXXXXX")
  if ! cat > "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  sync -f "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$dest"
}

atomic_copy() {  # atomic_copy <source> <destination>
  local source=$1 dest=$2 tmp
  mkdir -p "$(dirname "$dest")"
  tmp=$(mktemp "${dest}.tmp.XXXXXX")
  if ! cp -f -- "$source" "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  sync -f "$tmp" 2>/dev/null || true
  mv -f "$tmp" "$dest"
}

previous_base=""
if [[ -f "$BASE_REF_FILE" ]]; then
  previous_base=$(tr -d '[:space:]' < "$BASE_REF_FILE")
fi
if [[ -n ${LCCC_BASE_REF:-} ]]; then
  BASE=$(git rev-parse --verify "${LCCC_BASE_REF}^{commit}")
elif [[ -n $previous_base ]]; then
  BASE=$(git rev-parse --verify "${previous_base}^{commit}")
else
  # On a first snapshot, capture HEAD *before* committing pending work so the
  # generated patch contains that work rather than being empty.
  BASE=$(git rev-parse HEAD)
fi

# Refuse to commit a snapshot whose staged diff deletes tracked files or
# changes file modes, unless LCCC_SNAPSHOT_ALLOW_STRUCTURAL=1.
#
# Both are near-invisible accidents in this harness, and both have shipped:
#
#   * The harness wipes storage between turns and the restore strips exec
#     bits, so a following `git add -A` stages 100755 -> 100644 for every
#     script in the tree. Seventy-six mode-only hunks once made a 521-line
#     patch weigh 104 KiB.
#   * Five tracked files match patterns in this repo's own .gitignore
#     (`core.*` covers src/backend/{x86,i686}/assembler/encoder/core.rs;
#     `test_*` covers tests/integration/test_progressive.py and
#     tests/test_ivsr_indexed{.c,_before.s}). Once a wipe removes them,
#     `git add -A` stages the deletion and -- because ignored paths are
#     omitted from `git status` -- the tree reports CLEAN, so the loss stays
#     invisible until the published patch deletes two backend encoders.
#
# A published patch that deletes tracked files or drops exec bits is a
# restoration accident here, not an edit, so fail closed and keep the
# previous snapshot. Set LCCC_SNAPSHOT_ALLOW_STRUCTURAL=1 for a snapshot that
# genuinely removes or re-modes files.
# Refuse to publish against a stale base.
#
# `.base_ref` pins the upstream commit the deliverable is diffed from. After a
# rebase onto a newer upstream main it still names the OLD main, so the patch
# silently absorbs every upstream commit in between: a 6-file / 38 KiB series
# once published as 21 files / 143 KiB because PR #438's aggregate-SROA work
# was diffed as if it were ours. Nothing in the pipeline noticed, because the
# extra content is a legitimate diff -- it is just not this series.
#
# The invariant is cheap: the recorded base must be the merge base of HEAD and
# the fetched upstream main. When `origin/main` is unavailable (offline) the
# check is skipped rather than guessed. LCCC_SNAPSHOT_ALLOW_STALE_BASE=1
# overrides for a deliberate historical base.
snapshot_guard_base_is_merge_base() {
  if [[ ${LCCC_SNAPSHOT_ALLOW_STALE_BASE:-0} == 1 ]]; then
    return 0
  fi
  local upstream mb
  upstream=$(git rev-parse --verify --quiet origin/main) || return 0
  mb=$(git merge-base "$HEAD_SHA" "$upstream" 2>/dev/null) || return 0
  if [[ $mb != "$BASE" ]]; then
    printf 'snapshot guard: base %s is not the merge base of HEAD and origin/main (%s)\n' \
      "${BASE:0:12}" "${mb:0:12}" >&2
    printf '  The deliverable would absorb upstream commits as if they were ours.\n' >&2
    printf '  After a rebase, re-anchor:  printf %%s\\n %s > %s\n' "$mb" "$BASE_REF_FILE" >&2
    return 1
  fi
  return 0
}

snapshot_guard_staged_diff() {
  if [[ ${LCCC_SNAPSHOT_ALLOW_STRUCTURAL:-0} == 1 ]]; then
    echo "snapshot guard: structural changes explicitly allowed" >&2
    return 0
  fi
  local deleted modes bad=0
  deleted=$(git diff --cached --name-status | awk '$1 ~ /^D/ { print "    " $2 }')
  modes=$(git diff --cached --summary | grep 'mode change' | sed 's/^/    /')
  if [[ -n $deleted ]]; then
    echo "snapshot guard: staged diff DELETES tracked files:" >&2
    printf '%s\n' "$deleted" >&2
    bad=1
  fi
  if [[ -n $modes ]]; then
    echo "snapshot guard: staged diff changes file modes:" >&2
    printf '%s\n' "$modes" | head -5 >&2
    echo "    ($(printf '%s\n' "$modes" | wc -l) in total)" >&2
    bad=1
  fi
  if ((bad)); then
    cat >&2 <<'MSG'
  This is almost always a harness wipe: restore the paths and their modes
  before snapshotting, e.g.
      git checkout <upstream-main> -- <paths>
      git ls-files -s <paths>            # confirm 100755 where expected
  Note that ignored-but-tracked paths never reappear via `git add -A`.
MSG
    return 1
  fi
  return 0
}

if ! git diff --quiet || ! git diff --cached --quiet || \
   [[ -n "$(git ls-files --others --exclude-standard)" ]]; then
  git add -A
  snapshot_guard_staged_diff || exit 1
  git -c user.name='LCCC Agent' -c user.email='agent@lccc.local' \
      commit -m "$slug: $desc"
fi

HEAD_SHA=$(git rev-parse HEAD)
if ! git merge-base --is-ancestor "$BASE" "$HEAD_SHA"; then
  printf 'snapshot base %s is not an ancestor of HEAD %s\n' "$BASE" "$HEAD_SHA" >&2
  exit 1
fi
snapshot_guard_base_is_merge_base || exit 1

# Publish a requested/new base atomically only after it has been verified
# against the committed snapshot head. The per-entry ledger note below makes a
# base re-anchor auditable without rewriting historic entries.
if [[ $previous_base != "$BASE" ]]; then
  printf '%s\n' "$BASE" | atomic_write "$BASE_REF_FILE"
fi
base_note=""
if [[ -n $previous_base && $previous_base != "$BASE" ]]; then
  base_note="re-anchored from $previous_base to $BASE"
elif [[ -z $previous_base ]]; then
  base_note="initialized at $BASE"
fi

if [[ -f "$seq_file" ]]; then
  current_seq=$(tr -d '[:space:]' < "$seq_file")
  [[ $current_seq =~ ^[0-9]+$ ]] || { echo "invalid snapshot sequence: $current_seq" >&2; exit 1; }
else
  current_seq=0
fi
seq=$((current_seq + 1))
tag=$(printf 'S%02d-%s' "$seq" "$slug")

# ---- 1. Produce and verify the patch before publishing any new snapshot -----
patch_tmp=$(mktemp "$ART/.ms178-1.${tag}.patch.tmp.XXXXXX")
if [[ "$HEAD_SHA" != "$BASE" ]]; then
  git diff --binary --no-color "$BASE" "$HEAD_SHA" > "$patch_tmp"
else
  : > "$patch_tmp"
fi
bytes=$(wc -c < "$patch_tmp")
verdict="EMPTY"
if [[ "$bytes" -gt 0 ]]; then
  verify_worktree=$(mktemp -d "${TMPDIR:-/tmp}/lccc-snapshot-verify.XXXXXX")
  cleanup_verify() {
    git -C "$REPO" worktree remove --force "$verify_worktree" >/dev/null 2>&1 || true
    rm -rf "$verify_worktree"
  }
  if ! git -C "$REPO" worktree add --detach "$verify_worktree" "$BASE" >/dev/null; then
    rm -f "$patch_tmp"
    echo "unable to create verification worktree" >&2
    exit 1
  fi
  if ! git -C "$verify_worktree" apply --check "$patch_tmp"; then
    cleanup_verify
    rm -f "$patch_tmp"
    echo "generated patch does not apply cleanly to its recorded base" >&2
    exit 1
  fi
  cleanup_verify
  verdict="APPLIES-CLEAN"
fi

# ---- 2. Materialize the three patch copies atomically ------------------------
atomic_copy "$patch_tmp" "$DELIVERABLE"
atomic_copy "$patch_tmp" "$ART/ms178-1.patch"
atomic_copy "$patch_tmp" "$ART/ms178-1.${tag}.patch"
patch_sha=$(sha256sum "$patch_tmp" | awk '{print $1}')
rm -f "$patch_tmp"

# ---- 3. Per-commit series (reviewable history) -------------------------------
series_tmp=$(mktemp -d "$ART/.series.tmp.XXXXXX")
if ! git format-patch -q -o "$series_tmp" "$BASE..$HEAD_SHA" >/dev/null; then
  rm -rf "$series_tmp"
  echo "git format-patch failed; preserving prior series" >&2
  exit 1
fi
series_backup="$ART/.series.previous.${tag}"
rm -rf "$series_backup"
if [[ -e "$ART/series" ]]; then
  mv "$ART/series" "$series_backup"
fi
if ! mv "$series_tmp" "$ART/series"; then
  [[ -e "$series_backup" ]] && mv "$series_backup" "$ART/series" || true
  echo "unable to publish patch series; prior series restored when possible" >&2
  exit 1
fi
rm -rf "$series_backup"

# ---- 4. Full source tarball (fallback if patch application breaks) -----------
tar_tmp=$(mktemp "$ART/.lccc-src.tar.gz.tmp.XXXXXX")
if ! tar --exclude=.git --exclude=target --exclude=node_modules \
    -czf "$tar_tmp" -C "$(dirname "$REPO")" "$(basename "$REPO")"; then
  rm -f "$tar_tmp"
  echo "source tarball creation failed; preserving prior archive" >&2
  exit 1
fi
[[ -s "$tar_tmp" ]] || { rm -f "$tar_tmp"; echo "source tarball is empty" >&2; exit 1; }
sync -f "$tar_tmp" 2>/dev/null || true
mv -f "$tar_tmp" "$ART/lccc-src.tar.gz"
tar_sha=$(sha256sum "$ART/lccc-src.tar.gz" | awk '{print $1}')

# ---- 5. Git bundle (fallback preserving complete history) --------------------
bundle_tmp=$(mktemp "$ART/.lccc.bundle.tmp.XXXXXX")
if ! git bundle create "$bundle_tmp" --all; then
  rm -f "$bundle_tmp"
  echo "git bundle creation failed; preserving prior bundle" >&2
  exit 1
fi
[[ -s "$bundle_tmp" ]] || { rm -f "$bundle_tmp"; echo "git bundle is empty" >&2; exit 1; }
sync -f "$bundle_tmp" 2>/dev/null || true
mv -f "$bundle_tmp" "$ART/lccc.bundle"
bundle_sha=$(sha256sum "$ART/lccc.bundle" | awk '{print $1}')

# ---- 6. Atomically extend the ledger ------------------------------------------
ledger_tmp=$(mktemp "$ART/.SNAPSHOT_LEDGER.md.tmp.XXXXXX")
if [[ -f "$LEDGER" ]]; then
  cat "$LEDGER" > "$ledger_tmp"
else
  {
    echo "# LCCC Snapshot Ledger"
    echo
    echo "Initial base (upstream): \`$BASE\`"
    echo
    echo "| # | UTC | Tag | Description | Cumulative diffstat |"
    echo "|---|-----|-----|-------------|---------------------|"
  } > "$ledger_tmp"
fi
if [[ -n $base_note ]]; then
  printf '\n<!-- %s: %s -->\n' "$tag" "$base_note" >> "$ledger_tmp"
fi
files=$(git diff --stat "$BASE" "$HEAD_SHA" | tail -1 | sed 's/^ *//')
printf '| %d | %s | `%s` | %s | %s |\n' \
  "$seq" "$stamp" "$tag" "$desc" "${files:-none}" >> "$ledger_tmp"
printf '<!-- base=%s patch_sha256=%s tar_sha256=%s bundle_sha256=%s verdict=%s -->\n' \
  "$BASE" "$patch_sha" "$tar_sha" "$bundle_sha" "$verdict" >> "$ledger_tmp"
sync -f "$ledger_tmp" 2>/dev/null || true
mv -f "$ledger_tmp" "$LEDGER"

# Advance the sequence only once every independently recoverable artifact and
# the ledger have been published successfully.
printf '%s\n' "$seq" | atomic_write "$seq_file"
sync 2>/dev/null || true

echo "SNAPSHOT $tag"
echo "  base       : $BASE"
echo "  head       : $HEAD_SHA"
echo "  deliverable: $DELIVERABLE ($bytes bytes, sha256 $patch_sha) [$verdict]"
echo "  source tar : $ART/lccc-src.tar.gz (sha256 $tar_sha)"
echo "  bundle     : $ART/lccc.bundle (sha256 $bundle_sha)"
echo "  artifacts  : $ART"
