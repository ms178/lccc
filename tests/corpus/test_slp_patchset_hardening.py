#!/usr/bin/env python3
"""Executable red-team ratchets for the SLP patchset and its snapshot helper.

Hosted CI proved that a single `#[ignore]` raised the repository's
`ignored_tests` census and failed two independent lanes.  These tests pin the
root cause, the stronger malformed-seed guard, the zero-copy view design, and
the harness-wipe snapshot contract.  Every check is compiler-free so the fast
corpus lane can run it on both local and hosted CI.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SLP = REPO / "src/passes/slp_vectorizer.rs"
JOINT = REPO / "src/passes/slp_vectorizer/joint.rs"
SNAPSHOT = REPO / "engineering/verification/save_ms178_1_snapshot.sh"
AUDIT_CENSUS = REPO / "scripts/audit_census.py"


class SlpPatchsetHardening(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.slp = SLP.read_text()
        cls.joint = JOINT.read_text()
        cls.snapshot_script = SNAPSHOT.read_text()

    def test_no_ignored_rust_tests_in_the_slp_patchset(self) -> None:
        # Strip Rust line comments first: prose that explains the historical
        # defect must not be mistaken for a live ignored-test attribute.
        for source in (self.slp, self.joint):
            for line in source.splitlines():
                code = line.split("//", 1)[0]
                self.assertNotIn("#[ignore", code)

    def test_audit_census_is_green_on_the_live_tree(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(AUDIT_CENSUS), "--check"],
            cwd=REPO, capture_output=True, text=True, timeout=180,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertNotIn("audit census grew", proc.stdout + proc.stderr)

    def test_build_plan_rejects_out_of_bounds_store_positions(self) -> None:
        self.assertIn("if cand.store_idx.is_empty()", self.slp)
        self.assertIn("i >= block.instructions.len()", self.slp)
        self.assertIn("out-of-bounds or mismatched store/lane span", self.slp)

    def test_malformed_seed_matrix_includes_extreme_indices(self) -> None:
        self.assertIn("vec![0, 1, 2, 4096]", self.slp)
        self.assertIn("vec![usize::MAX]", self.slp)

    def test_view_trait_uses_borrowed_receivers_and_one_blanket_impl(self) -> None:
        for needle in (
            "pub(super) trait StreamEntryRef",
            "fn off(&self) -> i64",
            "fn val(&self) -> Operand",
            "impl<T: StreamEntryRef + ?Sized> StreamEntryRef for &T",
        ):
            self.assertIn(needle, self.slp)
        self.assertNotIn("fn off(self) -> i64", self.slp)
        self.assertIn("let mut last: Vec<&E>", self.slp)

    def test_site_partition_is_borrowed_and_singleton_work_is_skipped(self) -> None:
        self.assertIn("Vec<Vec<&'a StreamEntry>>", self.slp)
        self.assertIn("buckets[idx].push(e);", self.slp)
        self.assertNotIn("buckets[idx].push(e.clone())", self.slp)
        self.assertIn("if subset.len() < 2", self.slp)
        self.assertIn("candidates.extend(site_candidates)", self.slp)

    def test_snapshot_script_diffs_from_the_merge_base_not_head(self) -> None:
        self.assertIn('git -C "$ROOT" diff --binary "$base"', self.snapshot_script)
        self.assertIn("merge-base HEAD origin/main", self.snapshot_script)
        self.assertNotIn('git diff --binary HEAD >', self.snapshot_script)
        self.assertIn("HEAD^ fallback", self.snapshot_script)
        self.assertIn("MS178_1_NO_INTENT", self.snapshot_script)

    def test_snapshot_script_produces_a_reverse_appliable_full_delta(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ms178-snapshot-") as td:
            patch = Path(td) / "canonical.patch"
            manifest = Path(td) / "canonical.json"
            env = {
                **os.environ,
                "MS178_1_PATCH": str(patch),
                "MS178_1_MANIFEST": str(manifest),
                "MS178_1_NO_INTENT": "1",
            }
            proc = subprocess.run(
                ["bash", str(SNAPSHOT)], cwd=REPO, env=env,
                capture_output=True, text=True, timeout=180,
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertIn("SNAPSHOT_OK", proc.stdout)
            record = json.loads(manifest.read_text())
            self.assertEqual(record["sha256"], hashlib.sha256(patch.read_bytes()).hexdigest())
            self.assertEqual(record["bytes"], patch.stat().st_size)
            self.assertGreater(record["changed_file_count"], 0)
            self.assertTrue(
                record["base_mode"].startswith(
                    ("merge-base HEAD origin/main", "HEAD^ fallback", "HEAD fallback")
                ),
                record["base_mode"],
            )
            self.assertTrue(record["base"])
            reverse = subprocess.run(
                ["git", "apply", "--check", "-R", str(patch)],
                cwd=REPO, capture_output=True, text=True, timeout=180,
            )
            self.assertEqual(reverse.returncode, 0, reverse.stdout + reverse.stderr)

    def test_snapshot_contract_does_not_grow_the_ignored_census(self) -> None:
        # The live census is the authority; this source check names the exact
        # failure mode from hosted CI so it cannot be reintroduced silently.
        self.assertNotIn('ignore = "Callgrind', self.slp)
        self.assertNotIn('ignore = "Callgrind', self.joint)


if __name__ == "__main__":
    unittest.main()
