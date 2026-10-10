#!/usr/bin/env python3
"""Provision a revision-pinned GCC 17 development torture corpus, not a compiler.

The release provisioner remains scripts/ensure_gcc_torture.sh. This companion
resolves master ONCE, fetches that commit with a sparse checkout, verifies every
selected file against Git's blob IDs, and atomically replaces a damaged cache.
The old tree stays usable if fetching or validation fails. No upstream sources
are vendored into LCCC; the checkout belongs in the snapshot-excluded cache.

  python3 scripts/ensure_gcc_torture_git.py
  python3 scripts/ensure_gcc_torture_git.py --ref <full-commit-id>
  python3 scripts/x86_gcc_torture.py --suite \
    ~/.cache/lccc-gcc17/gcc/testsuite/gcc.c-torture/execute -j2

Save the printed commit and use --ref to reproduce; the default intentionally
tracks today's master. HTTPS/Git is the upstream trust boundary, not a release
signature. --repository permits a local mirror (also used by offline tests).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from tools.corpus import publication

REPOSITORY = "https://github.com/gcc-mirror/gcc.git"
TREES = ("gcc/testsuite/gcc.c-torture", "gcc/testsuite/gcc.dg", "gcc/testsuite/lib")
IDENTITY = ("gcc/BASE-VER",)


def git(root: Path, *args: str) -> bytes:
    return subprocess.check_output(
        ["git", "-C", str(root), *args], stderr=subprocess.PIPE, timeout=300,
        env=dict(os.environ, LC_ALL="C", GIT_TERMINAL_PROMPT="0"),
    )


def resolve(repository: str, ref: str) -> str:
    if re.fullmatch(r"[0-9a-fA-F]{40}", ref):
        return ref.lower()
    output = subprocess.check_output(
        ["git", "ls-remote", "--exit-code", repository, ref],
        stderr=subprocess.PIPE, timeout=60,
        env=dict(os.environ, LC_ALL="C", GIT_TERMINAL_PROMPT="0"),
    ).decode().splitlines()
    commits = {line.split()[0] for line in output}
    if len(commits) != 1:
        raise ValueError(f"ref must resolve unambiguously to one commit: {ref}")
    commit = commits.pop()
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("upstream did not return a SHA-1 commit ID")
    return commit


def verify(root: Path, commit: str, major: int) -> dict:
    if git(root, "rev-parse", "HEAD").decode().strip() != commit:
        raise ValueError("checkout HEAD differs from requested commit")
    version = (root / "gcc/BASE-VER").read_text().strip()
    if not re.fullmatch(rf"{major}\.\d+\.\d+", version):
        raise ValueError(f"expected GCC {major} development tree, found {version!r}")
    records = git(root, "ls-tree", "-rz", "HEAD", "--", *TREES, *IDENTITY)
    expected = set()
    manifest = hashlib.sha256()
    for record in records.split(b"\0"):
        if not record:
            continue
        meta, name = record.split(b"\t", 1)
        mode, kind, blob = meta.split()
        path = Path(os.fsdecode(name))
        if mode not in (b"100644", b"100755") or kind != b"blob":
            raise ValueError(f"non-regular source entry: {path}")
        file = root / path
        if file.is_symlink() or not file.is_file():
            raise ValueError(f"missing/non-regular source: {path}")
        data = file.read_bytes()
        actual = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        if actual != blob.decode():
            raise ValueError(f"source differs from Git object: {path}")
        manifest.update(name + b"\0" + hashlib.sha256(data).digest())
        expected.add(path)
    actual_paths = {Path(item) for item in IDENTITY}
    for tree in TREES:
        actual_paths.update(p.relative_to(root) for p in (root / tree).rglob("*")
                            if not p.is_dir() or p.is_symlink())
    if expected != actual_paths:
        raise ValueError("selected source file set differs from Git tree")
    counts = {}
    for leg in ("execute", "compile"):
        count = len(list((root / TREES[0] / leg).glob("*.c")))
        if not count:
            raise ValueError(f"empty {leg} corpus")
        counts[leg] = count
    return dict(commit=commit, version=version, manifest_sha256=manifest.hexdigest(),
                source_files=len(expected), counts=counts)


def write_stamp(root: Path, identity: dict) -> None:
    # The stamp is a report identity, never a substitute for verify(). Repair
    # it even on the warm-cache path (e.g. adopting an existing sparse clone).
    publication.atomic_bytes(root / "gcc/testsuite/.lccc-provisioned", (
        f"gcc-{identity['version']}-git-{identity['commit']}\n"
        f"manifest-sha256={identity['manifest_sha256']}\n"
    ).encode())


def provision(repository: str, ref: str, destination: Path, major: int = 17) -> dict:
    # Do not resolve symlinks away: publication must reject them, not overwrite
    # their targets. Concurrent writers share one lock beside the live tree.
    destination = destination.expanduser().absolute()
    destination.parent.mkdir(parents=True, exist_ok=True)
    commit = resolve(repository, ref)
    with publication.lock(destination.with_name(destination.name + ".lock")):
        if destination.is_symlink():
            raise ValueError("checkout destination must not be a symlink")
        if destination.exists():
            try:
                identity = verify(destination, commit, major)
                write_stamp(destination, identity)
                return dict(identity, repository=repository, suite=str(destination / TREES[0]))
            except (OSError, ValueError, subprocess.SubprocessError) as exc:
                print(f"re-provisioning corpus: {exc}", file=sys.stderr)
        stage = Path(tempfile.mkdtemp(prefix=".gcc17-stage-", dir=destination.parent))
        try:
            git(stage, "init", "-q")
            git(stage, "remote", "add", "origin", repository)
            git(stage, "config", "remote.origin.promisor", "true")
            git(stage, "config", "remote.origin.partialclonefilter", "blob:none")
            git(stage, "sparse-checkout", "init", "--cone")
            git(stage, "sparse-checkout", "set", *TREES)
            git(stage, "fetch", "-q", "--depth=1", "--filter=blob:none", "origin", commit)
            git(stage, "checkout", "-q", "--detach", "FETCH_HEAD")
            identity = verify(stage, commit, major)
            write_stamp(stage, identity)
            publication.publish_directory(stage, destination, force=destination.exists())
        finally:
            shutil.rmtree(stage, ignore_errors=True)
    return dict(identity, repository=repository, suite=str(destination / TREES[0]))


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--repository", default=REPOSITORY)
    parser.add_argument("--ref", default="refs/heads/master")
    parser.add_argument("--dest", type=Path, default=Path.home() / ".cache/lccc-gcc17")
    parser.add_argument("--major", type=int, default=17)
    args = parser.parse_args(argv)
    try:
        result = provision(args.repository, args.ref, args.dest, args.major)
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        detail = getattr(exc, "stderr", b"") or b""
        print(f"GCC git corpus failed: {exc}\n{detail.decode(errors='replace')[-2000:]}",
              file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
