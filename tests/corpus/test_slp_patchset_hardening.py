#!/usr/bin/env python3
"""Executable red-team ratchets for the SLP patchset and its snapshot helper.

Hosted CI proved that a single `#[ignore]` raised the repository's
`ignored_tests` census and failed two independent lanes.  It then proved that
a `fetch-depth: 1` checkout has neither `origin/main` ancestry nor `HEAD^`, so
a snapshot contract that silently assumed a deep, dirty local tree produced an
empty patch.  These tests pin the root causes, the stronger malformed-seed
guard, the zero-copy view design, and the harness-wipe snapshot contract with
isolated Git fixtures that behave identically on shallow hosted CI.  Every
check is compiler-free so the fast corpus lane can run it everywhere.
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


def _git(repo: Path, *args: str) -> None:
    subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )


def _make_repo(path: Path, tracked: str = "base\n") -> None:
    """A root-commit-only work tree: the shallow-CI shape under test."""
    path.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["git", "-c", "init.defaultBranch=main", "init", "-q"],
        cwd=path,
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )
    (path / "tracked.txt").write_text(tracked)
    _git(path, "add", "tracked.txt")
    subprocess.run(
        [
            "git",
            "-C",
            str(path),
            "-c",
            "user.name=Snapshot Contract",
            "-c",
            "user.email=snapshot@example.invalid",
            "commit",
            "-q",
            "-m",
            "base",
        ],
        check=True,
        capture_output=True,
        text=True,
        timeout=60,
    )


def _run_helper(
    root: Path,
    patch: Path,
    manifest: Path,
    *,
    no_intent: bool = False,
) -> subprocess.CompletedProcess[str]:
    env = {**os.environ}
    env["MS178_1_ROOT"] = str(root)
    env["MS178_1_PATCH"] = str(patch)
    env["MS178_1_MANIFEST"] = str(manifest)
    if no_intent:
        env["MS178_1_NO_INTENT"] = "1"
    else:
        env.pop("MS178_1_NO_INTENT", None)
    return subprocess.run(
        ["bash", str(SNAPSHOT)],
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        timeout=180,
    )


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
        for needle in (
            'git -C "$ROOT" diff --binary "$base"',
            "merge-base HEAD origin/main",
            "HEAD^ fallback",
            "HEAD fallback",
            "MS178_1_NO_INTENT",
            "MS178_1_ROOT",
            "rev-parse --is-inside-work-tree",
            "empty snapshot refused",
        ):
            self.assertIn(needle, self.snapshot_script)
        self.assertNotIn('git diff --binary HEAD >', self.snapshot_script)

    def test_snapshot_script_publishes_a_reverse_appliable_shallow_delta(self) -> None:
        """A shallow root-commit fixture with an untracked deliverable file."""
        with tempfile.TemporaryDirectory(prefix="ms178-snapshot-") as td:
            root = Path(td) / "repo"
            other = Path(td) / "forward"
            patch = Path(td) / "canonical.patch"
            manifest = Path(td) / "canonical.json"
            _make_repo(root)
            (root / "new-deliverable.txt").write_text("production delta\n")
            proc = _run_helper(root, patch, manifest)
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertIn("SNAPSHOT_OK", proc.stdout)
            record = json.loads(manifest.read_text())
            self.assertEqual(record["sha256"], hashlib.sha256(patch.read_bytes()).hexdigest())
            self.assertEqual(record["bytes"], patch.stat().st_size)
            self.assertGreater(record["bytes"], 0)
            self.assertGreater(record["changed_file_count"], 0)
            self.assertIn("new-deliverable.txt", record["changed_files"])
            self.assertTrue(record["base"])
            self.assertTrue(
                record["base_mode"].startswith("HEAD fallback"),
                record["base_mode"],
            )
            reverse = subprocess.run(
                ["git", "-C", str(root), "apply", "--check", "-R", str(patch)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(reverse.returncode, 0, reverse.stdout + reverse.stderr)
            # A forward apply to a second clean tree proves the patch carries
            # real content, not merely a reverse-apply-compatible index entry.
            _make_repo(other)
            forward = subprocess.run(
                ["git", "-C", str(other), "apply", str(patch)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(forward.returncode, 0, forward.stdout + forward.stderr)
            self.assertEqual(
                (other / "new-deliverable.txt").read_text(),
                "production delta\n",
            )

    def test_snapshot_script_no_intent_path_reverse_applies_a_tracked_edit(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ms178-no-intent-") as td:
            root = Path(td) / "repo"
            patch = Path(td) / "canonical.patch"
            manifest = Path(td) / "canonical.json"
            _make_repo(root)
            (root / "tracked.txt").write_text("modified\n")
            proc = _run_helper(root, patch, manifest, no_intent=True)
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertIn("SNAPSHOT_OK", proc.stdout)
            record = json.loads(manifest.read_text())
            self.assertGreater(record["bytes"], 0)
            self.assertIn("tracked.txt", record["changed_files"])
            reverse = subprocess.run(
                ["git", "-C", str(root), "apply", "--check", "-R", str(patch)],
                capture_output=True, text=True, timeout=60,
            )
            self.assertEqual(reverse.returncode, 0, reverse.stdout + reverse.stderr)

    def test_snapshot_script_refuses_empty_snapshots_fail_closed(self) -> None:
        """The exact hosted failure: shallow checkout plus clean work tree."""
        with tempfile.TemporaryDirectory(prefix="ms178-empty-") as td:
            root = Path(td) / "repo"
            patch = Path(td) / "canonical.patch"
            manifest = Path(td) / "canonical.json"
            _make_repo(root)
            proc = _run_helper(root, patch, manifest, no_intent=True)
            self.assertNotEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            combined = proc.stdout + proc.stderr
            self.assertIn("empty snapshot refused", combined)
            self.assertIn("reverse-apply check failed", combined)
            self.assertNotIn("SNAPSHOT_OK", proc.stdout)
            self.assertFalse(patch.exists())
            self.assertFalse(manifest.exists())

    def test_snapshot_script_rejects_a_non_git_root_override(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ms178-bad-root-") as td:
            root = Path(td) / "not-a-repo"
            root.mkdir()
            patch = Path(td) / "canonical.patch"
            manifest = Path(td) / "canonical.json"
            proc = _run_helper(root, patch, manifest, no_intent=True)
            self.assertNotEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertIn("not a Git work tree", proc.stdout + proc.stderr)
            self.assertFalse(patch.exists())
            self.assertFalse(manifest.exists())

    def test_snapshot_contract_does_not_grow_the_ignored_census(self) -> None:
        # The live census is the authority; this source check names the exact
        # failure mode from hosted CI so it cannot be reintroduced silently.
        self.assertNotIn('ignore = "Callgrind', self.slp)
        self.assertNotIn('ignore = "Callgrind', self.joint)


if __name__ == "__main__":
    unittest.main()
