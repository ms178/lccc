#!/usr/bin/env bash
# Save the current tree delta as the canonical harness-wipe-survival patch.
#
# Owner policy: the deliverable patch lives OUTSIDE the Git tree at
# /home/user/ms178-1.patch; ignored engineering/snapshots/ copies carry a
# UTC timestamp and the short HEAD so every validated fix has a sane name.
# The patch is reverse-apply checked before it is published, and the manifest
# records the exact digest, byte count, branch, HEAD and merge base.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
OUT=${MS178_1_PATCH:-/home/user/ms178-1.patch}
MANIFEST=${MS178_1_MANIFEST:-/home/user/ms178-1-snapshot-manifest.json}
SNAP_DIR="$ROOT/engineering/snapshots"
mkdir -p -- "$SNAP_DIR" "$(dirname -- "$OUT")"

# `git diff HEAD` includes staged AND unstaged tracked edits.  Intent-to-add
# new files (git add -N) are included too, so call this after staging the
# files that belong to the validated fix.
tmp="$OUT.tmp.$$"
git -C "$ROOT" diff --binary HEAD >"$tmp"
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

python3 - "$OUT" "$MANIFEST" "$SNAP_DIR/$name" "$ROOT" <<'PY'
import datetime
import hashlib
import json
import subprocess
import sys
from pathlib import Path

out, manifest, snapshot, root = map(Path, sys.argv[1:5])
data = out.read_bytes()


def git(*args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


changed = git("diff", "--name-only", "HEAD").splitlines()
record = {
    "file": str(out),
    "ignored_snapshot": str(snapshot),
    "bytes": len(data),
    "sha256": hashlib.sha256(data).hexdigest(),
    "branch": git("branch", "--show-current"),
    "head": git("rev-parse", "HEAD"),
    "base": git("merge-base", "HEAD", "origin/main"),
    "changed_files": changed,
    "changed_file_count": len(changed),
    "status": "validated working-tree delta; reverse-apply checked",
    "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
}
manifest.write_text(json.dumps(record, indent=2, sort_keys=True) + "\n")
print(
    f"SNAPSHOT_OK bytes={record['bytes']} sha256={record['sha256']} "
    f"files={record['changed_file_count']} snapshot={snapshot.name}"
)
PY
