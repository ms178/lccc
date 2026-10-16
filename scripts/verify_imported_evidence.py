#!/usr/bin/env python3
"""Verify imported historical evidence inventories (PR832 artifacts, PR835 import).

The canonical, scope-closed PR832 retention inventory is verified by
``scripts/verify_evidence_hashes.py``; this companion script covers the
schema-1 ``artifacts`` inventories brought in by later imports, including
deduplicated-path provenance rows. Integrity is not reproduction or proof of
a measurement's interpretation. External absolute paths inside historical
measurement JSON are not fabricated as available files. See
engineering/evidence/README.md for that boundary.
"""
from __future__ import annotations

import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
ADDITIONAL = Path("engineering/evidence/pr832/ARTIFACTS.json")
IMPORTED = Path("engineering/evidence/pr835-import-integrity.json")


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
    candidate = root.resolve()
    for part in path.parts:
        candidate /= part
        if candidate.is_symlink():
            raise ValueError("symlink artifact path: " + name)
    result = candidate.resolve(strict=True)
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
    # PR834's original inventory has a `files` list and no schema field.
    # Normalize its envelope, never regenerate its hashes or drop its raw hashes.
    if isinstance(data, dict) and "files" in data:
        if "schema" in data or "artifacts" in data:
            raise ValueError("ambiguous evidence inventory")
        data = {**data, "schema": 1, "artifacts": data["files"]}
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
        if "raw_sha256" in row:
            expected = row["raw_sha256"]
            if not isinstance(expected, str) or not re.fullmatch(r"[0-9a-f]{64}", expected):
                raise ValueError("invalid decoded SHA256: " + name)
            if name not in gzip_hashes:
                gzip_hashes[name], _ = digest(contained(root, name), compressed=True)
            if gzip_hashes[name] != expected:
                raise ValueError("decoded bytes mismatch: " + name)
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
            leaf = Path(row["gzip"])
            if len(leaf.parts) != 1 or leaf.name in (".", "..") or leaf.is_absolute():
                raise ValueError("raw-log member must be a filename")
            relative = str(path.parent.relative_to(root.resolve()) / row["gzip"])
            if relative not in seen:
                raise ValueError("compressed raw log not authenticated by inventory: " + relative)
            actual = gzip_hashes.get(relative)
            if actual is None:
                actual, _ = digest(contained(root, relative), compressed=True)
            if actual != row["raw_sha256"]:
                raise ValueError("decompressed raw-log mismatch: " + relative)
            raw_count += 1
    aliases = data.get("deduplicated", [])
    if not isinstance(aliases, list):
        raise ValueError("deduplicated inventory must be a list")
    originals = set()
    for alias in aliases:
        if not isinstance(alias, dict):
            raise ValueError("invalid deduplicated record")
        original, retained = alias.get("original"), alias.get("retained")
        if (not isinstance(original, str) or not original or Path(original).is_absolute()
                or ".." in Path(original).parts or original in seen or original in originals
                or not isinstance(retained, str) or retained not in seen):
            raise ValueError("invalid deduplicated paths")
        originals.add(original)
        # Original names are provenance, not filesystem redirects. No symlink,
        # alias traversal or second payload is needed to recover their bytes.
        if (root / original).exists() or (root / original).is_symlink():
            raise ValueError("deduplicated original still exists: " + original)
        actual, size = digest(contained(root, retained))
        if alias.get("sha256") != actual or type(alias.get("bytes")) is not int or alias["bytes"] != size:
            raise ValueError("deduplicated bytes mismatch: " + original)
    return {"artifacts": len(seen), "decompressed_raw_logs": raw_count,
            "gzip_streams": len(gzip_hashes), "deduplicated_paths": len(aliases)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--manifest", type=Path, help="one inventory (default: both imported inventories)")
    args = parser.parse_args(argv)
    try:
        manifests = [args.manifest] if args.manifest is not None else [ADDITIONAL, IMPORTED]
        result = {str(name): verify(args.root.resolve(), contained(args.root, str(name)))
                  for name in manifests}
    except (OSError, ValueError, KeyError, TypeError, EOFError) as exc:
        print("imported-evidence-integrity: FAIL:", exc)
        return 1
    print("imported-evidence-integrity: PASS", json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
