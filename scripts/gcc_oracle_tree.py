#!/usr/bin/env python3
"""Content/mode identity of an extracted GCC oracle or its pinned tar archive.

Hard links are represented as regular files with identical content; their inode
identity is irrelevant to compilation. Symlinks are hashed as links, never
followed. The prefix's own mode and timestamps/owners are intentionally ignored.
The archive digest must be verified *before* using its calculated tree digest
as a trust anchor. This does not identify host headers, binutils, CRT, or shared
libraries outside the prefix.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import stat
import sys
import tarfile


def file_digest(file) -> str:
    digest = hashlib.sha256()
    for block in iter(lambda: file.read(1024 * 1024), b""):
        digest.update(block)
    return digest.hexdigest()


def tree_digest(records: dict[str, tuple]) -> str:
    # JSON lines avoid ambiguity between names, modes and link targets. ASCII
    # encoding handles arbitrary filesystem names consistently through surrogate
    # escapes; timestamps, ownership and hardlink inode layout are not inputs.
    h = hashlib.sha256()
    for path, value in sorted(records.items()):
        h.update((json.dumps([path, *value], ensure_ascii=True, separators=(",", ":")) + "\n").encode("ascii"))
    return h.hexdigest()


def directory_records(root: Path) -> dict[str, tuple]:
    if root.is_symlink() or not root.is_dir():
        raise ValueError("oracle prefix must be a real directory")
    records = {}
    def walk(directory: Path):
        with os.scandir(directory) as entries:
            for entry in entries:
                path = Path(entry.path)
                name = path.relative_to(root).as_posix()
                info = entry.stat(follow_symlinks=False)
                mode = stat.S_IMODE(info.st_mode)
                if stat.S_ISLNK(info.st_mode):
                    records[name] = ("link", os.readlink(path))
                elif stat.S_ISDIR(info.st_mode):
                    records[name] = ("dir", mode)
                    walk(path)
                elif stat.S_ISREG(info.st_mode):
                    with path.open("rb") as stream:
                        records[name] = ("file", mode, file_digest(stream))
                else:
                    raise ValueError(f"unsupported oracle entry: {name}")
    walk(root)
    return records


def archive_records(archive: Path) -> dict[str, tuple]:
    records: dict[str, tuple] = {}
    hardlinks: dict[str, tuple[str, int]] = {}
    with tarfile.open(archive, "r:xz") as tar:
        for member in tar:
            parts = PurePosixPath(member.name).parts
            if not parts or parts[0] != "gcc-16" or any(p in (".", "..") for p in parts):
                raise ValueError(f"unsafe archive path: {member.name!r}")
            if len(parts) == 1:
                if not member.isdir():
                    raise ValueError("archive root is not a directory")
                continue
            name = PurePosixPath(*parts[1:]).as_posix()
            if name in records or name in hardlinks:
                raise ValueError(f"duplicate archive path: {name}")
            mode = member.mode & 0o7777
            if member.isdir():
                records[name] = ("dir", mode)
            elif member.issym():
                records[name] = ("link", member.linkname)
            elif member.isfile():
                stream = tar.extractfile(member)
                if stream is None:
                    raise ValueError(f"unreadable archive entry: {name}")
                with stream:
                    records[name] = ("file", mode, file_digest(stream))
            elif member.islnk():
                target = PurePosixPath(member.linkname).parts
                if not target or target[0] != "gcc-16" or any(p in (".", "..") for p in target):
                    raise ValueError(f"unsafe hardlink target: {member.linkname!r}")
                hardlinks[name] = (PurePosixPath(*target[1:]).as_posix(), mode)
            else:
                raise ValueError(f"unsupported archive entry: {name}")
    # GCC's hardlinks refer to ordinary files. Reject unresolved or chained
    # links rather than silently assigning a different identity.
    for name, (target, mode) in hardlinks.items():
        value = records.get(target)
        if value is None or value[0] != "file" or value[1] != mode:
            raise ValueError(f"unresolved or incompatible hardlink: {name}")
        records[name] = value
    if not records:
        raise ValueError("empty oracle tree")
    return records


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--archive", type=Path)
    group.add_argument("--directory", type=Path)
    parser.add_argument("--expect", help="fail if the computed digest differs")
    args = parser.parse_args()
    try:
        records = archive_records(args.archive) if args.archive else directory_records(args.directory)
        digest = tree_digest(records)
        if args.expect is not None and digest != args.expect:
            raise ValueError(f"oracle tree mismatch: expected {args.expect}, got {digest}")
        print(digest)
        return 0
    except (OSError, ValueError, tarfile.TarError) as exc:
        print(f"gcc oracle tree: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
