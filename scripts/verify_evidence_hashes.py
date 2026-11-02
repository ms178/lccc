#!/usr/bin/env python3
"""Verify retained evidence inventories, raw-log hashes and bounded file coverage.

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
import zlib

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = Path("engineering/evidence/pr832-integrity.json")
ADDITIONAL = Path("engineering/evidence/pr832/ARTIFACTS.json")
FOLLOWUPS = Path("engineering/evidence/pr835/ARTIFACTS.json")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON key: " + key)
        result[key] = value
    return result


def read_json(path):
    return _json(path)


def contained(root, name):
    if not isinstance(name, str) or not name:
        raise ValueError("artifact path must be a nonempty string")
    safe_name(name)
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
    try:
        with (gzip.open(path, "rb") if compressed else path.open("rb")) as stream:
            size, value = _digest(stream, MAX_DECODED if compressed else MAX_STORED)
        return value, size
    except (OSError, EOFError, zlib.error) as error:
        raise ValueError(f"invalid evidence stream: {path}: {error}") from error


SCOPE = "engineering/evidence/pr832-integrity.scope.json"
CHUNK = 1024 * 1024
MAX_DECODED = 64 * CHUNK
MAX_STORED = 256 * CHUNK
MAX_JSON = 4 * CHUNK


def safe_name(name, *, leaf=False):
    if (not isinstance(name, str) or not name or "\\" in name or ":" in name or "\0" in name
            or any(p in {"", ".", ".."} for p in name.split("/"))
            or (leaf and "/" in name)):
        raise ValueError(f"unsafe evidence {'filename' if leaf else 'path'}: {name!r}")
    return name


def _json(path):
    with path.open("rb") as stream:
        raw = stream.read(MAX_JSON + 1)
    if len(raw) > MAX_JSON:
        raise ValueError(f"metadata exceeds {MAX_JSON} bytes: {path}")
    return json.loads(raw, object_pairs_hook=unique_object)


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


def verify_legacy(root, manifest, scope=SCOPE):
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
    return {"artifacts": len(rows), "decompressed_raw_logs": len(decoded), "gzip_streams": len(decoded)}



def verify(root, manifest, scope=SCOPE):
    data = read_json(manifest)
    # The legacy inventory has its own strict schema and independent scope.
    # Preserve those checks instead of normalizing away their requirements.
    if isinstance(data, dict) and "files" in data:
        if "schema" in data or "artifacts" in data:
            raise ValueError("ambiguous evidence inventory")
        return verify_legacy(root, manifest, scope)
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
            safe_name(row["gzip"], leaf=True)
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
    aliases = data.get("canonical_aliases", [])
    if not isinstance(aliases, list):
        raise ValueError("canonical aliases must be a list")
    former_paths = set()
    for alias in aliases:
        if not isinstance(alias, dict):
            raise ValueError("invalid canonical alias")
        former = alias.get("former_path")
        canonical = alias.get("canonical_path")
        if (not isinstance(former, str) or not former or Path(former).is_absolute()
                or ".." in Path(former).parts or former in former_paths or former in seen):
            raise ValueError("invalid/duplicate former artifact path")
        former_paths.add(former)
        if (root / former).exists() or (root / former).is_symlink():
            raise ValueError("redundant artifact was reintroduced: " + former)
        if canonical not in seen:
            raise ValueError("canonical alias target is not authenticated")
        actual, size = digest(contained(root, canonical))
        if alias.get("sha256") != actual or type(alias.get("bytes")) is not int or alias["bytes"] != size:
            raise ValueError("canonical alias bytes differ from original")
    if "coverage_roots" in data or "coverage_files" in data:
        roots, files = data.get("coverage_roots", []), data.get("coverage_files", [])
        if (not isinstance(roots, list) or not isinstance(files, list)
                or not all(isinstance(n, str) and n for n in roots + files)
                or len(set(roots)) != len(roots) or len(set(files)) != len(files)):
            raise ValueError("invalid coverage scope")
        scoped = set(files)
        for name in files:
            contained(root, name)
        for name in roots:
            relative = Path(name)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError("unsafe coverage root")
            directory = root.resolve()
            for part in relative.parts:
                directory /= part
                if directory.is_symlink():
                    raise ValueError("symlink coverage root")
            if not directory.is_dir():
                raise ValueError("missing coverage root")
            for path in directory.rglob("*"):
                if path.is_symlink():
                    raise ValueError("symlink within coverage root")
                if path.is_file() and path.resolve() != manifest.resolve():
                    scoped.add(str(path.relative_to(root.resolve())))
        if not scoped or scoped != seen:
            raise ValueError("inventory coverage mismatch: missing=" + str(sorted(scoped-seen))
                             + " outside=" + str(sorted(seen-scoped)))
    return {"artifacts": len(seen), "decompressed_raw_logs": raw_count,
            "gzip_streams": len(gzip_hashes), "canonical_aliases": len(aliases)}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--manifest", type=Path, help="one inventory (default: all three retained inventories)")
    parser.add_argument("--scope", default=SCOPE, help="independent legacy inventory scope")
    args = parser.parse_args(argv)
    try:
        manifests = [args.manifest] if args.manifest is not None else [DEFAULT, ADDITIONAL, FOLLOWUPS]
        result = {str(name): verify(args.root.resolve(), contained(args.root, str(name)), args.scope)
                  for name in manifests}
    except (OSError, ValueError, KeyError, TypeError, EOFError) as exc:
        print("evidence-hash-integrity: FAIL:", exc)
        return 1
    print("evidence-hash-integrity: PASS", json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
