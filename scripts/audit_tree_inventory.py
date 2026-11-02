#!/usr/bin/env python3
"""Inventory every tracked blob at an immutable Git revision (not a semantic audit).

Uses one git ls-tree traversal, so builds, untracked logs and worktree edits
do not contaminate the count. Output includes source/script paths, blob identities,
subsystem counts, and a hash of the complete file manifest. Never infers a bug
from a TODO, unwrap, file length or missing keyword.
"""
from __future__ import annotations

import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


def bucket(path: str) -> str:
    parts = path.split("/")
    if parts[0] == "src" and len(parts) > 2:
        if parts[1] == "backend" and len(parts) > 3:
            return "/".join(parts[:3])
        return "/".join(parts[:2])
    return parts[0] if len(parts) > 1 else "root"


def inventory(repo: Path, revision: str) -> dict:
    commit = subprocess.check_output(
        ["git", "-C", str(repo), "rev-parse", "--verify", revision + "^{commit}"],
        text=True).strip()
    tree = subprocess.check_output(["git", "-C", str(repo), "ls-tree", "-rlz", commit])
    groups: dict = defaultdict(lambda: {"files": 0, "bytes": 0})
    paths = []
    submodules = []
    for entry in tree.split(b"\0"):
        if not entry:
            continue
        meta, raw_path = entry.split(b"\t", 1)
        mode, kind, oid, size = meta.split()
        path = raw_path.decode("utf-8", "surrogateescape")
        if kind != b"blob":
            submodules.append(path)
            continue
        groups[bucket(path)]["files"] += 1
        groups[bucket(path)]["bytes"] += int(size)
        paths.append({"path": path, "blob": oid.decode(), "bytes": int(size),
                      "mode": mode.decode()})
    # Identify exact Rust and script coverage without publishing thousands of
    # fixture rows. The complete Git-tree manifest is authenticated below.
    detail = [row for row in paths if row["path"].startswith(("src/", "scripts/", "tools/", "include/", ".github/"))]
    return {"schema": "lccc-tree-inventory-v1", "commit": commit,
            "manifest_sha256": hashlib.sha256(tree).hexdigest(),
            "scope": "tracked blob inventory only; not line-by-line semantic verification",
            "tracked_blobs": len(paths), "submodules_not_traversed": submodules,
            "rust_files": sum(row["path"].endswith(".rs") for row in paths),
            "groups": dict(sorted(groups.items())), "source_and_tool_manifest": detail}


class InventoryTests(unittest.TestCase):
    def test_immutable_revision_and_unusual_paths(self):
        with tempfile.TemporaryDirectory(prefix="audit-inventory-test-") as temp:
            root = Path(temp)
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            (root / "src").mkdir()
            (root / "src/demo.rs").write_text("fn main() {}\n")
            (root / "name with space\tand newline\n.txt").write_text("fixture\n")
            subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
            subprocess.run(["git", "-C", str(root), "-c", "user.name=Inventory Test",
                            "-c", "user.email=inventory@example.invalid", "commit",
                            "--quiet", "-m", "fixture"], check=True)
            first = inventory(root, "HEAD")
            self.assertEqual(first["tracked_blobs"], 2)
            self.assertEqual(first["rust_files"], 1)
            self.assertEqual(first["groups"]["src"]["bytes"], 13)
            (root / "src/demo.rs").write_text("modified but not committed\n")
            (root / "untracked").write_text("not inventory input\n")
            self.assertEqual(inventory(root, "HEAD"), first)

    def test_buckets(self):
        examples = {"Cargo.toml": "root", "src/lib.rs": "src",
                    "src/pgo/mod.rs": "src/pgo",
                    "src/backend/x86/linker/elf.rs": "src/backend/x86",
                    "src/backend/traits.rs": "src/backend",
                    "tests/regression/a.c": "tests", "scripts/a.py": "scripts"}
        for path, expected in examples.items():
            self.assertEqual(bucket(path), expected)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=ROOT)
    parser.add_argument("--revision", default="HEAD")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--selftest", action="store_true")
    args = parser.parse_args()
    if args.selftest:
        tests = unittest.defaultTestLoader.loadTestsFromTestCase(InventoryTests)
        return int(not unittest.TextTestRunner(verbosity=2).run(tests).wasSuccessful())
    text = json.dumps(inventory(args.repo, args.revision), indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text)
    else:
        print(text, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
