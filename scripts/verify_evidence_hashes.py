#!/usr/bin/env python3
"""Read-only integrity check of the retained PR832 evidence, not a rerun claim."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = "engineering/evidence/pr832-integrity.json"


def contained(root, name):
    path = Path(name)
    if path.is_absolute() or ".." in path.parts or not path.parts:
        raise ValueError(f"unsafe evidence path: {name}")
    candidate = root / path
    if candidate.is_symlink() or not candidate.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"escaping/symlink evidence path: {name}")
    return candidate


def verify(root, manifest):
    data = json.loads(manifest.read_text())
    rows = data["files"]
    if not rows:
        raise ValueError("empty evidence inventory")
    seen = set()
    for row in rows:
        name = row["path"]
        if name in seen:
            raise ValueError(f"duplicate evidence entry: {name}")
        seen.add(name)
        raw = contained(root, name).read_bytes()
        if len(raw) != row["bytes"] or hashlib.sha256(raw).hexdigest() != row["sha256"]:
            raise ValueError(f"stored bytes mismatch: {name}")
        if "raw_sha256" in row:
            decoded = gzip.decompress(raw)
            if hashlib.sha256(decoded).hexdigest() != row["raw_sha256"]:
                raise ValueError(f"decoded bytes mismatch: {name}")
    # Cross-check the ORIGINAL recorded hashes, not just today's new inventory.
    for relative in data["raw_log_manifests"]:
        path = contained(root, relative)
        if relative not in seen:
            raise ValueError(f"uncovered raw-log manifest: {relative}")
        for row in json.loads(path.read_text()):
            leaf = Path(row["gzip"])
            if len(leaf.parts) != 1:
                raise ValueError("raw-log member must be a filename")
            name = str(path.parent.relative_to(root) / leaf)
            if name not in seen:
                raise ValueError(f"uncovered raw log: {name}")
            raw = gzip.decompress(contained(root, name).read_bytes())
            if hashlib.sha256(raw).hexdigest() != row["raw_sha256"]:
                raise ValueError(f"original raw-log hash mismatch: {name}")
    return len(rows)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--manifest", default=DEFAULT)
    args = parser.parse_args(argv)
    try:
        count = verify(args.root, contained(args.root, args.manifest))
    except (OSError, ValueError, KeyError, TypeError, EOFError) as error:
        print(f"evidence-hash-integrity: FAIL: {error}", file=sys.stderr)
        return 1
    print(f"evidence-hash-integrity: PASS ({count} files; integrity, not measurement truth)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
