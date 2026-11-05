#!/usr/bin/env python3
"""The published toolchain cache must contain only CURRENT, pinned tools.

A previous publisher run failed because it referenced an undefined
`$CLANG_VERSION` and never staged a Clang payload; the tag it would have
produced also carried retired binaries. These guards make the contract
structural: the publisher must derive both oracle versions from the single
enforcing installer, stage GDB 18.1 with the ISA contract, register it, and
refuse to publish any obsolete payload by construction.
"""

from __future__ import annotations

import ast
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
PUBLISHER = ROOT / "scripts/publish_agent_toolchain_cache.sh"
LOCK = ROOT / "engineering/toolchains/agent-cache-lock.json"
CLANG_INSTALLER = ROOT / "scripts/ci_install_clang_oracle.sh"
GDB_INSTALLER = ROOT / "scripts/ci_install_gdb_oracle.sh"

OBSOLETE = ("1.97.0", "23.1.4", "11.1.2", "v7.3-x86_64", "2.42.1")


def publisher_text() -> str:
    return PUBLISHER.read_text(encoding="utf-8")


def embedded_python() -> list[str]:
    return re.findall(r"<<'PY'\n(.*?)\nPY\n", publisher_text(), re.S)


class PublisherContract(unittest.TestCase):
    def test_embedded_python_blocks_parse(self) -> None:
        blocks = embedded_python()
        self.assertGreaterEqual(len(blocks), 4)
        for block in blocks:
            ast.parse(block)

    def test_clang_version_is_derived_from_the_single_live_pin(self) -> None:
        text = publisher_text()
        self.assertIn("CLANG_VERSION=$(sed -n 's/^VERSION=", text)
        self.assertIn("scripts/ci_install_clang_oracle.sh", text)
        self.assertIn("bash scripts/ci_install_clang_oracle.sh --prefix", text)
        self.assertIn('clang-$CLANG_VERSION-linux-x64.tar.xz', text)

    def test_staged_clang_is_identity_checked(self) -> None:
        text = publisher_text()
        self.assertIn('staged_clang=$("$WORK/clang-prefix/bin/clang" -dumpversion)', text)
        self.assertIn('[ "$staged_clang" = "$CLANG_VERSION" ]', text)

    def test_gdb_is_derived_staged_and_isa_checked(self) -> None:
        text = publisher_text()
        self.assertIn("GDB_VERSION=$(sed -n 's/^GDB_VERSION=", text)
        self.assertIn("scripts/ci_install_gdb_oracle.sh", text)
        self.assertIn('gdb-$GDB_VERSION-linux-x64.tar.xz', text)
        self.assertIn("grep -qx 'march=haswell'", text)
        self.assertIn("GNU gdb (GDB) $GDB_VERSION", text)

    def test_gdb_is_registered_in_the_cache_manifest(self) -> None:
        text = publisher_text()
        self.assertIn("('gdb',gdb_version,f'gdb-{gdb_version}-linux-x64.tar.xz')", text)
        self.assertIn("'gdb':f'GNU GDB", text)
        self.assertIn("'gdb','binutils'", text)  # priority list

    def test_obsolete_payload_guard_is_present_and_complete(self) -> None:
        text = publisher_text()
        self.assertIn("refusing to publish obsolete cache payloads", text)
        self.assertIn("banned_names={'rust-msrv','clang-tag-tree'}", text)
        for needle in OBSOLETE:
            self.assertIn(needle, text, f"guard does not cover {needle}")

    def test_guard_actually_rejects_an_obsolete_payload(self) -> None:
        # Execute the guard's own logic against a synthetic CACHE.json, so a
        # guard that merely looks convincing cannot pass.
        blocks = embedded_python()
        guard = next(b for b in blocks if "refusing to publish obsolete" in b)
        ns = {"__name__": "__guard__"}
        # Feed it a clean cache first, then an obsolete one, via sys.argv.
        import sys
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            clean = {"tools": {"gdb": {"file": "gdb-18.1-linux-x64.tar.xz"}},
                     "priority": ["gdb"]}
            (d / "CACHE.json").write_text(json.dumps(clean))
            old = sys.argv
            try:
                sys.argv = ["guard", str(d / "CACHE.json"), str(d)]
                exec(compile(guard, "guard", "exec"), ns)  # must not raise
                bad = {"tools": {"clang": {"file": "clang-23.1.4-linux-x64.tar.xz"}},
                       "priority": ["clang"]}
                (d / "CACHE.json").write_text(json.dumps(bad))
                with self.assertRaises(SystemExit):
                    exec(compile(guard, "guard", "exec"), ns)
            finally:
                sys.argv = old

    def test_publisher_is_diagnosable_on_failure(self) -> None:
        text = publisher_text()
        self.assertIn("trap on_error ERR", text)
        self.assertIn("::error title=agent-cache publisher::", text)

    def test_default_tag_is_the_refreshed_cache(self) -> None:
        self.assertIn("TAG=${TAG:-harness-cache-8}", publisher_text())

    def test_reviewed_lock_has_no_obsolete_payload(self) -> None:
        lock = json.loads(LOCK.read_text(encoding="utf-8"))
        self.assertEqual(lock["tag"], "harness-cache-8")
        self.assertIn("gdb", lock["tools"])
        self.assertEqual(lock["tools"]["gdb"]["version"], "18.1")
        self.assertEqual(lock["tools"]["clang"]["version"], "23.1.5")
        self.assertEqual(lock["tools"]["qemu"]["version"], "11.1.3")
        self.assertEqual(lock["tools"]["kernel-relocs"]["version"], "7.3.1")
        self.assertNotIn("rust-msrv", lock["tools"])
        for name, spec in lock["tools"].items():
            for needle in OBSOLETE:
                self.assertNotIn(needle, spec["file"], f"{name} carries {needle}")
        for field in ("tag_sha", "commit_sha", "tree_sha", "cache_json_sha256"):
            self.assertTrue(lock.get(field), field)
        self.assertEqual(len(lock["tag_sha"]), 40)
        self.assertEqual(len(lock["cache_json_sha256"]), 64)


if __name__ == "__main__":
    unittest.main()
