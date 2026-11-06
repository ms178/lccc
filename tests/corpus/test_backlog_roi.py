#!/usr/bin/env python3
"""Regression suite for the 20-item backlog ROI ratchet.

The ratchet is only worth running if every one of its 20 checks has teeth.
These tests pin the item count, the live-tree verdict, the CLI contract, the
negative controls, and the measurement ledger that records the evidence.
"""

from __future__ import annotations

import copy
import json
import subprocess
import sys
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))

from scripts import check_backlog_roi as roi  # noqa: E402

REPORT = REPO / "engineering/BACKLOG-ROI-2026-11-05.md"


class BacklogRoiRatchet(unittest.TestCase):
    def setUp(self) -> None:
        self.ctx = roi.Context.load()

    def test_exactly_twenty_unique_items(self) -> None:
        names = [name for name, _ in roi.CHECKS]
        self.assertEqual(len(names), 20)
        self.assertEqual(len(set(names)), 20)

    def test_every_check_passes_on_the_live_tree(self) -> None:
        results = roi.run_all(self.ctx)
        failures = {name: problems for name, problems in results.items() if problems}
        self.assertEqual(failures, {})

    def test_every_check_has_a_negative_control(self) -> None:
        self.assertEqual(roi.negative_controls(self.ctx), [])

    def test_documented_table_has_twenty_rows(self) -> None:
        text = REPORT.read_text()
        rows = [line for line in text.splitlines()
                if line.startswith("| ") and len(line.split("|")) > 3
                and line.split("|")[1].strip().isdigit()]
        self.assertEqual(len(rows), 20, f"measurement table has {len(rows)} rows")
        for row in rows:
            fields = [f.strip() for f in row.split("|")[1:-1]]
            self.assertEqual(len(fields), 6, row)
            self.assertTrue(all(fields), row)

    def test_every_check_name_is_documented(self) -> None:
        text = REPORT.read_text()
        for name, _ in roi.CHECKS:
            self.assertIn(name, text, f"{name} has no measurement row")

    def test_cli_json_reports_twenty_passes(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(REPO / "scripts/check_backlog_roi.py"),
             "--self-test", "--json"],
            capture_output=True, text=True, timeout=120,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(report["items"], 20)
        self.assertEqual(report["passed"], 20)
        self.assertEqual(report["failed"], [])
        self.assertEqual(report["self_test"], [])

    def test_a_regressed_ledger_fails_the_first_check(self) -> None:
        ctx = copy.copy(self.ctx)
        ctx.ledger_rows = list(self.ctx.ledger_rows) + [self.ctx.ledger_rows[0]]
        self.assertTrue(roi.run_all(ctx)["GP-22/1"])

    def test_a_stale_simd_provenance_fails_its_check(self) -> None:
        ctx = copy.copy(self.ctx)
        ctx.matrix = {
            **self.ctx.matrix,
            "provenance": {
                **self.ctx.matrix.get("provenance", {}),
                "rederived_with": {"clang": "23.1.4"},
            },
        }
        self.assertTrue(roi.run_all(ctx)["FOLLOWUP-7/2"])

    def test_an_obsolete_cache_payload_fails_its_check(self) -> None:
        ctx = copy.copy(self.ctx)
        lock = copy.deepcopy(self.ctx.agent_lock)
        lock["tools"]["clang"] = {
            **lock["tools"].get("clang", {}),
            "file": "clang-23.1.4-linux-x64.tar.xz",
        }
        ctx.agent_lock = lock
        self.assertTrue(roi.run_all(ctx)["CACHE-REDTEAM/1"])

    def test_catch_all_arm_is_detected_only_in_live_code(self) -> None:
        """The historical blanket arm is documented in a cli.rs NOTE.  A
        comment must not trip the ratchet, but live code must."""
        self.assertEqual(roi.run_all(self.ctx)["MB-40"], [])
        ctx = copy.copy(self.ctx)
        ctx.cli = self.ctx.cli + '\n                arg if arg.starts_with("-f") => {}\n'
        self.assertTrue(roi.run_all(ctx)["MB-40"])

    def test_context_loads_every_input(self) -> None:
        for field in ("ledger_rows", "cli", "driver_gate", "macro_py", "macro_sh",
                      "capability", "matrix", "matrix_gate", "godbolt", "agent_lock",
                      "backlog", "ci_local", "ci_workflow", "clang_pin",
                      "predefines", "publisher", "isa_doc"):
            with self.subTest(field=field):
                self.assertTrue(getattr(self.ctx, field), field)


if __name__ == "__main__":
    unittest.main()
