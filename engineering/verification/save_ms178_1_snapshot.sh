#!/usr/bin/env bash
# Save the CURRENT BRANCH DELTA as the canonical harness-wipe-survival patch.
#
# Owner policy: the deliverable patch lives OUTSIDE the Git tree at
# /home/user/ms178-1.patch; ignored engineering/snapshots/ copies carry a
# UTC timestamp and the short HEAD so every validated fix has a sane name.
#
# `git diff HEAD` was a defective contract after commits landed: it wrote an
# EMPTY patch from a clean tree.  The snapshot must instead be the full delta
# from the merge base with origin/main through HEAD *and* the working tree.
# On a shallow CI checkout where merge-base is unavailable, fall back to the
# first parent of HEAD, which still captures the latest commit plus all
# uncommitted work.  The patch is reverse-apply checked before publication.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
OUT=${MS178_1_PATCH:-/home/user/ms178-1.patch}
MANIFEST=${MS178_1_MANIFEST:-/home/user/ms178-1-snapshot-manifest.json}
SNAP_DIR="$ROOT/engineering/snapshots"
mkdir -p -- "$SNAP_DIR" "$(dirname -- "$OUT")" "$(dirname -- "$MANIFEST")"

# New deliverable files must appear in `git diff <base>`.  Intent-to-add does
# not stage their contents; it only makes the diff show them.  Ignored build
# artifacts and timestamped snapshot copies are excluded by Git itself.
# MS178_1_NO_INTENT=1 is for read-only contract tests on an already clean
# tree; production snapshot calls leave it unset.
if [[ ${MS178_1_NO_INTENT:-0} != 1 ]]; then
    git -C "$ROOT" ls-files --others --exclude-standard -z |
        xargs -0 -r git -C "$ROOT" add -N --
fi

if base=$(git -C "$ROOT" merge-base HEAD origin/main 2>/dev/null) && [[ -n $base ]]; then
    base_mode="merge-base HEAD origin/main"
else
    if parent=$(git -C "$ROOT" rev-parse HEAD^ 2>/dev/null); then
        base=$parent
        base_mode="HEAD^ fallback (shallow checkout; origin/main merge-base unavailable)"
    else
        base=$(git -C "$ROOT" rev-parse HEAD)
        base_mode="HEAD fallback (root commit; working-tree delta only)"
    fi
fi

tmp="$OUT.tmp.$$"
git -C "$ROOT" diff --binary "$base" >"$tmp"
if ! git -C "$ROOT" apply --check -R "$tmp"; then
    rm -f -- "$tmp"
    echo "save_ms178_1_snapshot.sh: reverse-apply check failed" >&2
    exit 1
fi
mv -f -- "$tmp" "$OUT"

stamp=$(date -u +%Y%m%dT%H%M%SZ)
short=$(git -C "$ROOT" rev-parse --short HEAD)
name="ms178-1-${stamp}-${short}.patch"
cp -f -- "$OUT" "$SNAP_DIR/$name"

python3 - "$OUT" "$MANIFEST" "$SNAP_DIR/$name" "$ROOT" "$base" "$base_mode" <<'PY'
import datetime
import hashlib
import json
import subprocess
import sys
from pathlib import Path

out, manifest, snapshot, root, base, base_mode = sys.argv[1:7]
out, manifest, snapshot, root = map(Path, (out, manifest, snapshot, root))
data = out.read_bytes()


def git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


changed = sorted(set(git("diff", "--name-only", base).splitlines()))
record = {
    "file": str(out),
    "ignored_snapshot": str(snapshot),
    "bytes": len(data),
    "sha256": hashlib.sha256(data).hexdigest(),
    "branch": git("branch", "--show-current"),
    "head": git("rev-parse", "HEAD"),
    "base": base,
    "base_mode": base_mode,
    "changed_files": changed,
    "changed_file_count": len(changed),
    "status": "full base-to-working-tree delta; reverse-apply checked",
    "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
}
manifest.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
print(
    f"SNAPSHOT_OK bytes={record['bytes']} sha256={record['sha256']} "
    f"files={record['changed_file_count']} base={base[:12]} snapshot={snapshot.name}"
)
PY
