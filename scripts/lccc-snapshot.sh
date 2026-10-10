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
# successful call.  The snapshot also records a reviewable format-patch series
# and ledger under /home/user/artifacts, an upstream-base-dependent session bundle (the original session
# commits, with an explicit BASE prerequisite and independent restore proof) under /home/user/artifacts as well, and a compressed tracked-source archive in the same persisted zone.
# There is one uncompressed patch, a bounded compressed checkpoint history,
# and a compressed patch series (no exploding loose-file count).
# ms178-1.patch.zip is automatically refreshed and byte/hash verified; the
# visible workspace is capped at 64 MiB / 256 files by the publisher.
#
# Usage:  ./lccc-snapshot.sh "<slug>" "<one-line description>"
# Env:    LCCC_REPO, LCCC_ARTIFACTS, LCCC_DELIVERABLE
#         LCCC_CI_STAMP — ci_local.sh pass stamp (default target/ci_local.pass);
#                         a mode=fast or mode=full stamp for this tree is accepted.
#                         the snapshot refuses a tree without a matching
#                         ci_local.sh --fast pass is the local gate; GitHub CI
#                         runs the rest.
#         LCCC_SNAPSHOT_ALLOW_PARTIAL=1 — accept a --fast/--slow stamp for an
#                         interim autosave (ledger: *-PARTIAL-NOT-DELIVERABLE).
#         LCCC_SNAPSHOT_UNGATED=1 — emergency pre-wipe save without that proof
#                         (ledger ci_gate=UNGATED; never a delivery candidate).
#         LCCC_BASE_REF — explicit upstream base for a rebase/new session.
#                         When set, it atomically supersedes artifacts/.base_ref
#                         after ancestry validation.
# ============================================================================
set -euo pipefail

# The checkout this script lives in.  LCCC_REPO still overrides, but the default
# must work from any clone: the deliverable is produced by running this script
# from the tree it describes.
REPO=${LCCC_REPO:-$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)}
ART=${LCCC_ARTIFACTS:-/home/user/artifacts}
# Source archive and review series are persisted as SINGLE compressed files.
# The worktree, EDG checkout and toolchains belong in the excluded .cache zone.
# A source archive must survive a wipe of that zone, not merely be derivable
# from the worktree that just vanished. BULK remains an explicit override.
BULK=${LCCC_BULK_ARTIFACTS:-$ART}
mkdir -p "$BULK"
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
# Serialize sequence allocation, commit and publication. Never let two writers
# generate a checkpoint with the same tag or overwrite each other's ledger.
exec 9>"$ART/.snapshot.lock"
flock -n 9 || { echo "another snapshot writer is active" >&2; exit 4; }
KEEP=${LCCC_SNAPSHOT_KEEP:-2}
[[ $KEEP =~ ^[1-9][0-9]*$ ]] || { echo "LCCC_SNAPSHOT_KEEP must be positive" >&2; exit 2; }
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
  chmod 644 "$tmp"
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
  chmod 644 "$tmp"
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
  python3 "$REPO/scripts/snapshot_policy.py"
}

# ---- CI-mirror gate -----------------------------------------------------------
# A snapshot is a delivery candidate, so it must carry proof that EXACTLY this
# tree passed the local CI mirror (S20 shipped a red CI because ci_local.sh was
# skipped). ci_local.sh writes the git tree hash of the worktree it tested --
# re-checked at the end, so an edit during the run voids it -- and we compare
# it with the tree `git add -A` is about to commit. Running the 30+ minute
# mirror here instead would defeat the point of a fast, wipe-resistant
# autosave. LCCC_SNAPSHOT_UNGATED=1 exists for emergency pre-wipe saves only
# and is recorded as such in the ledger.
CI_STAMP=${LCCC_CI_STAMP:-$REPO/target/ci_local.pass}
tree_now=$(bash "$REPO/scripts/worktree_tree.sh" "$REPO") ||
  { echo "unable to hash the worktree" >&2; exit 1; }
# Read only an existing stamp: under `set -euo pipefail` a failing sed in the
# substitution would abort the script without the explanation below.
stamp_tree='' stamp_mode='' stamp_os=''
if [[ -r $CI_STAMP ]]; then
  stamp_tree=$(sed -n 's/^tree=//p' "$CI_STAMP" | head -1)
  stamp_mode=$(sed -n 's/^mode=//p' "$CI_STAMP" | head -1)
  stamp_os=$(sed -n 's/^os=//p' "$CI_STAMP" | head -1)
fi
# Policy (owner decision): a local `--fast` pass is the snapshot gate. GitHub CI
# runs the slow gates on every PR, so the local full run is not required here.
if [[ -n $stamp_tree && $stamp_tree == "$tree_now" && ( $stamp_mode == fast || $stamp_mode == full ) ]]; then
  # Recorded, not enforced: GitHub's ubuntu-latest moves (24.04 -> 26.04 on
  # 2026-10-19) and not every host can run the chroot mirror. A non-Ubuntu
  # userland is shown in the ledger so a reviewer knows what was mirrored.
  ci_gate="ci_local-${stamp_mode}-PASS@${stamp_os:-unknown-os}"
  if [[ ${stamp_os:-} != ubuntu-* ]]; then
    echo "WARNING: gates ran on ${stamp_os:-an unrecorded userland}, not the Ubuntu" \
         "runner's; scripts/ci_ubuntu_chroot.sh -- bash scripts/ci_local.sh --fast mirrors it." >&2
  fi
elif [[ -n $stamp_tree && $stamp_tree == "$tree_now" &&
        ${LCCC_SNAPSHOT_ALLOW_PARTIAL:-0} == 1 ]]; then
  # A --fast (or --slow) stamp covers only part of what GitHub runs: S24 went
  # red on PR #638 in check_peephole_whitespace.sh, a gate --fast skips, after
  # this script had accepted its fast stamp. Partial stamps are therefore only
  # an interim autosave, labelled as such in the ledger, never a delivery.
  ci_gate="ci_local-${stamp_mode:-unknown}-PARTIAL-NOT-DELIVERABLE"
  echo "WARNING: tree $tree_now has only a mode=${stamp_mode:-unknown} stamp;" \
       "recorded as $ci_gate." >&2
elif [[ ${LCCC_SNAPSHOT_UNGATED:-0} == 1 ]]; then
  ci_gate="UNGATED"
  echo "WARNING: snapshot of tree $tree_now WITHOUT a matching ci_local.sh pass;" \
       "recorded as UNGATED -- not a delivery candidate." >&2
else
  cat >&2 <<MSG
refusing to snapshot: tree $tree_now has no matching ci_local.sh --fast pass
  stamp: $CI_STAMP (tree=${stamp_tree:-none}, mode=${stamp_mode:-none})
Run ./scripts/ci_local.sh --fast on this exact tree (GitHub CI runs the slow gates).
Interim autosave with a partial stamp: LCCC_SNAPSHOT_ALLOW_PARTIAL=1.
Emergency pre-wipe save without any stamp: LCCC_SNAPSHOT_UNGATED=1.
MSG
  exit 3
fi

# Generated metadata is a source contract, independent of compiler CI stamps.
# Even an UNGATED save must not publish stale declarations after a core change.
if [[ -f "$REPO/tests/corpus/clang-c/corpus-index.json" ]]; then
  python3 "$REPO/scripts/edg_corpus_mine.py" verify-index || {
    echo 'snapshot refused: corpus declaration/integrity contract failed' >&2
    exit 1
  }
fi

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
seq=$((10#$current_seq + 1))
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

# ---- 2. One visible patch, compressed checkpoint history --------------------
# Full uncompressed copies grow as O(checkpoints * patch size) and exhausted
# Arena's ~128 MiB snapshot cap for the EDG import. Only the canonical patch is
# uncompressed. Checkpoints are bounded, gzip -n deterministic and verified.
atomic_copy "$patch_tmp" "$DELIVERABLE"
patch_sha=$(sha256sum "$patch_tmp" | awk '{print $1}')
checkpoint_tmp=$(mktemp "$ART/.checkpoint.tmp.XXXXXX")
gzip -n -9 < "$patch_tmp" > "$checkpoint_tmp"
gzip -t "$checkpoint_tmp"
sync -f "$checkpoint_tmp" 2>/dev/null || true
chmod 644 "$checkpoint_tmp"
mv -f "$checkpoint_tmp" "$ART/ms178-1.${tag}.patch.gz"
rm -f "$patch_tmp"

# ---- 3. A compact reviewable series, not thousands of loose files -----------
series_tmp=$(mktemp -d "$BULK/.series.tmp.XXXXXX")
git format-patch -q -o "$series_tmp" "$BASE..$HEAD_SHA" >/dev/null
series_archive=$(mktemp "$BULK/.series.tar.gz.tmp.XXXXXX")
tar -czf "$series_archive" -C "$series_tmp" .
gzip -t "$series_archive"
sync -f "$series_archive" 2>/dev/null || true
chmod 644 "$series_archive"
mv -f "$series_archive" "$BULK/series.tar.gz"
rm -rf "$series_tmp"

# ---- 4. Full source tarball (fallback if patch application breaks) -----------
# mktemp IN THE DESTINATION DIRECTORY: the rename below must stay within one
# filesystem to be atomic (audit F8 — a cross-fs mv is copy+unlink, and a kill
# mid-copy would publish a truncated tarball, breaking the header invariant).
tar_tmp=$(mktemp "$BULK/.lccc-src.tar.gz.tmp.XXXXXX")
# Archive the validated COMMIT, not a worktree containing logs/build caches.
if ! git archive --format=tar --prefix=lccc/ "$HEAD_SHA" | gzip -n > "$tar_tmp"; then
  rm -f "$tar_tmp"
  echo "source archive creation failed; preserving prior archive" >&2
  exit 1
fi
gzip -t "$tar_tmp"
[[ -s "$tar_tmp" ]] || { rm -f "$tar_tmp"; echo "source tarball is empty" >&2; exit 1; }
sync -f "$tar_tmp" 2>/dev/null || true
chmod 644 "$tar_tmp"
mv -f "$tar_tmp" "$BULK/lccc-src.tar.gz"
tar_sha=$(sha256sum "$BULK/lccc-src.tar.gz" | awk '{print $1}')
# A small authenticated offline recipe makes the compact history recoverable
# even without network/BASE objects: reverse the exact cumulative patch from
# the HEAD source, verify BASE's tree, then write its original commit bytes.
# This is independently atomic metadata, not a multi-file transaction claim.
python3 - "$REPO" "$BASE" "$HEAD_SHA" "$tar_sha" "$patch_sha" "$DELIVERABLE" <<'PYSOURCE' | atomic_write "$ART/SOURCE_ARCHIVE.json"
import base64,json,subprocess,sys
repo,base,head,archive_sha,patch_sha,patch=sys.argv[1:]
def git(*args):return subprocess.check_output(['git','-C',repo,*args])
raw=git('cat-file','commit',base)
if len(raw)>65536:raise ValueError('BASE commit exceeds offline recipe budget')
print(json.dumps(dict(format='lccc-source-archive-v2',archive='lccc-src.tar.gz',archive_sha256=archive_sha,
    patch=__import__('pathlib').Path(patch).name,patch_sha256=patch_sha,prefix='lccc/',head=head,
    tree=git('rev-parse',head+'^{tree}').decode().strip(),branch=git('branch','--show-current').decode().strip(),
    base=base,base_commit_base64=base64.b64encode(raw).decode(),
    offline_exact_history='source+patch+BASE recipe+matching session bundle; source-only fallback explicitly lacks original history'),
    sort_keys=True,allow_nan=False))
PYSOURCE


# ---- 5. Exact session history, not all upstream/remote history --------------
# A --all bundle contributed ~35 MiB of largely unrelated history to every
# workspace scan. Preserve original BASE..HEAD commits in a verified ~2 MiB
# incremental bundle instead. Its BASE prerequisite is explicit in the receipt;
# lccc-src.tar.gz is the complete offline source fallback.
bundle_receipt=$(python3 "$REPO/scripts/session_bundle.py" \
  --repo "$REPO" --base "$BASE" --output "$ART/lccc-session.bundle")
bundle_sha=$(printf '%s' "$bundle_receipt" | python3 -c \
  'import json,sys; print(json.load(sys.stdin)["bundle_sha256"])')
bundle_clone=ISOLATED-BASE-RESTORE-OK
printf '%s\n' "$bundle_receipt" | atomic_write "$ART/SESSION_BUNDLE.json"
# Standalone standard-library consumer must survive the /opt worktree reset.
# It needs no installed Python package, Rust, compiler or imported source tree.
if [[ -f "$REPO/scripts/lccc_recover.py" ]]; then
  atomic_copy "$REPO/scripts/lccc_recover.py" "$(dirname "$DELIVERABLE")/LCCC-RECOVER.py"
fi

# Prune owned compressed checkpoints BEFORE the final-output budget check.
# Each removed checkpoint is superseded by a newer full cumulative patch.
python3 - "$ART" "$KEEP" <<'PYKEEP'
from pathlib import Path
import re, sys
root, keep = Path(sys.argv[1]), int(sys.argv[2])
items = sorted(root.glob('ms178-1.S*.patch.gz'),
               key=lambda p: int(re.search(r'\.S(\d+)-', p.name)[1]))
for p in items[:-keep]: p.unlink()
PYKEEP

# ---- 6. User-facing, verified compact download -----------------------------
# Avoid previewing hundreds of thousands of diff lines. The ZIP carries the
# exact canonical patch plus base/head/hash/status and is opened as a download.
# This is a transport receipt, NOT a compiler-correctness or CI certificate.
python3 "$REPO/scripts/lccc_delivery.py" \
  --patch "$DELIVERABLE" --workspace "$(dirname "$DELIVERABLE")" \
  --base "$BASE" --head "$HEAD_SHA" --snapshot "$tag" --ci-gate "$ci_gate"

# ---- 7. Atomically extend the ledger ------------------------------------------
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
printf '<!-- base=%s patch_sha256=%s tar_sha256=%s bundle_sha256=%s bundle_clone=%s verdict=%s ci_gate=%s tree=%s -->\n' \
  "$BASE" "$patch_sha" "$tar_sha" "$bundle_sha" "${bundle_clone:-unknown}" "$verdict" "$ci_gate" "$tree_now" >> "$ledger_tmp"
# Same rule as atomic_write: mktemp's 0600 must not leak onto published
# artifacts. The mode must be fixed on the TEMP file BEFORE the rename —
# chmod after the mv publishes a 0600 ledger to every reader for the
# window between the two, and a crash in that window strands it: the
# rename is the atomic publish, so it must be the LAST step.
chmod 644 "$ledger_tmp"
sync -f "$ledger_tmp" 2>/dev/null || true
mv -f "$ledger_tmp" "$LEDGER"

# Advance the sequence only once every independently recoverable artifact and
# the ledger have been published successfully.
printf '%s\n' "$seq" | atomic_write "$seq_file"
# Keep the latest compressed checkpoints. Ledger hashes remain permanent.
python3 - "$ART" "$KEEP" <<'PYKEEP'
from pathlib import Path
import re, sys
root, keep = Path(sys.argv[1]), int(sys.argv[2])
items = sorted(root.glob('ms178-1.S*.patch.gz'),
               key=lambda p: int(re.search(r'\.S(\d+)-', p.name)[1]))
for p in items[:-keep]:
    p.unlink()
PYKEEP
sync 2>/dev/null || true

echo "SNAPSHOT $tag"
echo "  base       : $BASE"
echo "  head       : $HEAD_SHA"
echo "  deliverable: $DELIVERABLE ($bytes bytes, sha256 $patch_sha) [$verdict, $ci_gate]"
echo "  source tar : $BULK/lccc-src.tar.gz (sha256 $tar_sha)"
echo "  bundle     : $ART/lccc-session.bundle (requires upstream base) (sha256 $bundle_sha)"
echo "  artifacts  : $ART"
echo "  bulk       : $BULK"
