#!/usr/bin/env python3
"""Verify the explicitly inventoried PR832 artifacts and original raw-log hashes.

Integrity is not reproduction or proof of a measurement's interpretation.
External absolute paths inside historical measurement JSON are not fabricated
as available files. See engineering/evidence/README.md for that boundary.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = Path("engineering/evidence/pr832/ARTIFACTS.json")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key: " + key)
        result[key] = value
    return result


def read_json(path):
    return json.loads(path.read_text(), object_pairs_hook=unique_object)


def contained(root, name):
    if not isinstance(name, str) or not name:
        raise ValueError("artifact path must be a nonempty string")
    path = Path(name)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError("unsafe artifact path: " + str(name))
    result = (root / path).resolve(strict=True)
    if not result.is_relative_to(root.resolve()) or not result.is_file():
        raise ValueError("artifact is not a contained file: " + str(name))
    return result


def digest(path, compressed=False):
    value = hashlib.sha256()
    size = 0
    with (gzip.open(path, "rb") if compressed else path.open("rb")) as stream:
        while chunk := stream.read(1 << 20):
            size += len(chunk)
            if size > 128 << 20:
                raise ValueError("artifact exceeds bounded verification size")
            value.update(chunk)
    return value.hexdigest(), size


def verify(root, manifest):
    data = read_json(manifest)
    if not isinstance(data, dict) or type(data.get("schema")) is not int or data["schema"] != 1:
        raise ValueError("schema-1 inventory object required")
    if not isinstance(data.get("artifacts"), list) or not data["artifacts"]:
        raise ValueError("nonempty artifact list required")
    raw_manifests = data.get("raw_log_manifests", [])
    if not isinstance(raw_manifests, list) or not all(isinstance(n, str) for n in raw_manifests):
        raise ValueError("raw-log manifest paths must be a list of strings")
    if len(set(raw_manifests)) != len(raw_manifests):
        raise ValueError("duplicate raw-log manifest")
    seen = set()
    gzip_hashes = {}
    for row in data["artifacts"]:
        if (not isinstance(row, dict) or not isinstance(row.get("path"), str)
                or not isinstance(row.get("sha256"), str)
                or type(row.get("bytes")) is not int or row["bytes"] < 0):
            raise ValueError("invalid artifact record")
        name = row["path"]
        if name in seen:
            raise ValueError("duplicate artifact: " + name)
        seen.add(name)
        if not re.fullmatch(r"[0-9a-f]{64}", row["sha256"]):
            raise ValueError("invalid SHA256: " + name)
        actual, size = digest(contained(root, name))
        if actual != row["sha256"] or size != row["bytes"]:
            raise ValueError("artifact mismatch: " + name)
        if name.endswith(".gz"):
            gzip_hashes[name], _ = digest(contained(root, name), compressed=True)
    raw_count = 0
    for name in raw_manifests:
        if name not in seen:
            raise ValueError("raw manifest not authenticated by inventory: " + name)
        path = contained(root, name)
        rows = read_json(path)
        if not isinstance(rows, list) or not rows:
            raise ValueError("empty/invalid raw-log manifest: " + name)
        raw_seen = set()
        for row in rows:
            if (not isinstance(row, dict) or not isinstance(row.get("gzip"), str)
                    or not isinstance(row.get("raw_sha256"), str)
                    or not re.fullmatch(r"[0-9a-f]{64}", row["raw_sha256"])):
                raise ValueError("invalid raw-log record")
            if row["gzip"] in raw_seen:
                raise ValueError("duplicate raw log: " + row["gzip"])
            raw_seen.add(row["gzip"])
            relative = str(path.parent.relative_to(root.resolve()) / row["gzip"])
            if relative not in seen:
                raise ValueError("compressed raw log not authenticated by inventory: " + relative)
            actual = gzip_hashes.get(relative)
            if actual is None:
                actual, _ = digest(contained(root, relative), compressed=True)
            if actual != row["raw_sha256"]:
                raise ValueError("decompressed raw-log mismatch: " + relative)
            raw_count += 1
    return {"artifacts": len(seen), "decompressed_raw_logs": raw_count, "gzip_streams": len(gzip_hashes)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--manifest", type=Path, default=DEFAULT)
    args = parser.parse_args(argv)
    try:
        manifest = contained(args.root, str(args.manifest))
        result = verify(args.root.resolve(), manifest)
    except (OSError, ValueError, KeyError, TypeError) as exc:
        print("evidence-hash-integrity: FAIL:", exc)
        return 1
    print("evidence-hash-integrity: PASS", json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
