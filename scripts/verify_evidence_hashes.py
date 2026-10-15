#!/usr/bin/env python3
"""Bounded, read-only evidence integrity; neither authenticity nor a rerun claim."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = "engineering/evidence/pr832-integrity.json"
SCOPE = "engineering/evidence/pr832-integrity.scope.json"
CHUNK = 1024 * 1024
MAX_DECODED = 64 * CHUNK
MAX_STORED = 256 * CHUNK
MAX_JSON = 4 * CHUNK


def safe_name(name, *, leaf=False):
    if (not isinstance(name, str) or not name or "\\" in name or ":" in name or "\0" in name
            or any(p in {"", ".", ".."} for p in name.split("/"))
            or (leaf and "/" in name)):
        raise ValueError(f"unsafe evidence path: {name!r}")
    return name


def contained(root, name):
    safe_name(name)
    candidate = Path(root).resolve()
    for part in name.split("/"):
        candidate /= part
        if candidate.is_symlink():
            raise ValueError(f"symlink evidence component: {name}")
    return candidate


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _json(path):
    with path.open("rb") as stream:
        raw = stream.read(MAX_JSON + 1)
    if len(raw) > MAX_JSON:
        raise ValueError(f"metadata exceeds {MAX_JSON} bytes: {path}")
    return json.loads(raw, object_pairs_hook=_pairs)


def _keys(value, required, optional=()):
    if not isinstance(value, dict) or not set(required) <= value.keys() or value.keys() - set(required) - set(optional):
        raise ValueError(f"invalid object keys; required={sorted(required)}")


def _text(value):
    if not isinstance(value, str) or not value:
        raise ValueError("expected a nonempty string")


def _hex(value, width=64):
    if not isinstance(value, str) or re.fullmatch(rf"[0-9a-f]{{{width}}}", value) is None:
        raise ValueError(f"expected {width} lowercase hexadecimal digits")


def _version(value):
    if type(value) is not int or value != 1:
        raise ValueError("unsupported evidence schema_version")


def _names(value, *, nonempty=False):
    if not isinstance(value, list) or (nonempty and not value):
        raise ValueError("expected a nonempty path list" if nonempty else "expected a path list")
    for name in value:
        safe_name(name)
    if len(set(value)) != len(value):
        raise ValueError("duplicate path")
    return set(value)


def _digest(stream, limit):
    count = 0
    digest = hashlib.sha256()
    while block := stream.read(min(CHUNK, limit - count + 1)):
        count += len(block)
        if count > limit:
            raise ValueError(f"stream exceeds {limit} bytes")
        digest.update(block)
    return count, digest.hexdigest()


def _decoded(path):
    try:
        with gzip.open(path, "rb") as stream:
            return _digest(stream, MAX_DECODED)[1]
    except (OSError, EOFError, zlib.error) as error:
        raise ValueError(f"invalid gzip evidence: {path}: {error}") from error


def verify(root, manifest, scope=SCOPE):
    root = Path(root).resolve()
    manifest = contained(root, Path(manifest).absolute().relative_to(root).as_posix())
    data = _json(manifest)
    _keys(data, {"schema_version", "capture_commit", "scope", "raw_log_manifests", "files"})
    _version(data["schema_version"])
    _hex(data["capture_commit"], 40)
    _text(data["scope"])
    raw_manifests = _names(data["raw_log_manifests"])
    inventory = _json(contained(root, scope))
    _keys(inventory, {"schema_version", "description", "paths"})
    _version(inventory["schema_version"])
    _text(inventory["description"])
    required = _names(inventory["paths"], nonempty=True)
    rows = data["files"]
    if not isinstance(rows, list) or not rows:
        raise ValueError("empty/malformed evidence inventory")
    seen, decoded = set(), {}
    for row in rows:
        _keys(row, {"path", "bytes", "sha256"}, {"raw_sha256"})
        name = safe_name(row["path"])
        if name in seen:
            raise ValueError(f"duplicate evidence entry: {name}")
        seen.add(name)
        if type(row["bytes"]) is not int or not 0 <= row["bytes"] <= MAX_STORED:
            raise ValueError(f"invalid stored-byte count: {name}")
        _hex(row["sha256"])
        if name.endswith(".gz") and "raw_sha256" not in row:
            raise ValueError(f"missing decoded hash: {name}")
        if "raw_sha256" in row:
            _hex(row["raw_sha256"])
        path = contained(root, name)
        with path.open("rb") as stream:
            size, digest = _digest(stream, MAX_STORED)
        if size != row["bytes"] or digest != row["sha256"]:
            raise ValueError(f"stored bytes mismatch: {name}")
        if "raw_sha256" in row:
            decoded[name] = _decoded(path)
            if decoded[name] != row["raw_sha256"]:
                raise ValueError(f"decoded bytes mismatch: {name}")
    if seen != required:
        raise ValueError(f"incomplete scope: missing={sorted(required-seen)}, extra={sorted(seen-required)}")
    # Cross-check the ORIGINAL recorded hashes, not merely today's inventory.
    for relative in sorted(raw_manifests):
        if relative not in seen:
            raise ValueError(f"uncovered raw-log manifest: {relative}")
        path = contained(root, relative)
        records = _json(path)
        if not isinstance(records, list) or not records:
            raise ValueError(f"empty/malformed raw-log manifest: {relative}")
        members = set()
        for row in records:
            _keys(row, {"original", "gzip", "raw_sha256"})
            safe_name(row["original"], leaf=True)
            leaf = safe_name(row["gzip"], leaf=True)
            _hex(row["raw_sha256"])
            name = (Path(relative).parent / leaf).as_posix()
            if name in members:
                raise ValueError(f"duplicate raw-log member: {name}")
            members.add(name)
            if name not in seen:
                raise ValueError(f"uncovered raw log: {name}")
            if name not in decoded:
                decoded[name] = _decoded(contained(root, name))
            if decoded[name] != row["raw_sha256"]:
                raise ValueError(f"original raw-log hash mismatch: {name}")
    return len(rows)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--manifest", default=DEFAULT)
    parser.add_argument("--scope", default=SCOPE)
    args = parser.parse_args(argv)
    try:
        count = verify(args.root, contained(args.root, args.manifest), args.scope)
    except (OSError, ValueError, KeyError, TypeError, EOFError) as error:
        print(f"evidence-hash-integrity: FAIL: {error}", file=sys.stderr)
        return 1
    print(f"evidence-hash-integrity: PASS ({count} files; integrity, not measurement truth)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
