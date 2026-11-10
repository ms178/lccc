#!/usr/bin/env python3
"""Portable CI contract for the 16-item dynamic backlog ROI sweep.

The live sweep invokes a built compiler, so it belongs in the measured local
evidence run.  This module is the compiler-free half: it pins the item
registry, the check verdicts, the negative controls, the CLI JSON contract and
the measurement document.  It is discovered by the existing fast corpus lane
in both `scripts/ci_local.sh` and `.github/workflows/ci.yml`.
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

from scripts import check_backlog_roi_dynamic as roi  # noqa: E402

REPORT = REPO / "engineering/BACKLOG-ROI-DYNAMIC-2026-11-10.md"
SCRIPT = REPO / "scripts/check_backlog_roi_dynamic.py"


class BacklogRoiDynamicRatchet(unittest.TestCase):
    def setUp(self) -> None:
        self.observations = roi.synthetic()

    def test_exactly_sixteen_unique_items(self) -> None:
        names = [name for name, _ in roi.CHECKS]
        self.assertEqual(len(names), 16)
        self.assertEqual(len(set(names)), 16)
        self.assertEqual(names, [item[0] for item in roi.ITEMS])

    def test_at_least_ten_items_have_root_backlog_roi_at_least_four(self) -> None:
        high = [(item[0], item[1]) for item in roi.ITEMS if item[1] >= 4.0]
        self.assertGreaterEqual(len(high), 10, high)

    def test_synthetic_observations_pass_every_check(self) -> None:
        results = roi.run_checks(self.observations)
        self.assertEqual({k: v for k, v in results.items() if v}, {})

    def test_every_check_has_a_live_negative_control(self) -> None:
        self.assertEqual(roi.negative_controls(self.observations), [])

    def test_documented_table_has_sixteen_rows(self) -> None:
        text = REPORT.read_text()
        rows = [line for line in text.splitlines()
                if line.startswith("| ") and len(line.split("|")) > 3
                and line.split("|")[1].strip().isdigit()]
        self.assertEqual(len(rows), 16, f"measurement table has {len(rows)} rows")
        for row in rows:
            fields = [f.strip() for f in row.split("|")[1:-1]]
            self.assertEqual(len(fields), 6, row)
            self.assertTrue(all(fields), row)

    def test_every_check_name_is_documented(self) -> None:
        text = REPORT.read_text()
        for name, _ in roi.CHECKS:
            self.assertIn(name, text, f"{name} has no measurement row")

    def test_offline_cli_json_reports_sixteen_passes(self) -> None:
        proc = subprocess.run(
            [sys.executable, str(SCRIPT), "--offline", "--self-test", "--json"],
            capture_output=True, text=True, timeout=120,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        report = json.loads(proc.stdout)
        self.assertEqual(report["items"], 16)
        self.assertEqual(report["passed"], 16)
        self.assertEqual(report["failed"], [])
        self.assertEqual(report["self_test"], [])
        self.assertEqual(report["tools"]["mode"], "offline synthetic verdict machinery")

    def test_an_inert_fno_plt_fails_its_check(self) -> None:
        observations = copy.deepcopy(self.observations)
        observations["MB-01/1"]["flag_gotpcrel"] = 0
        self.assertTrue(roi.run_checks(observations)["MB-01/1"])

    def test_a_wrong_elf_class_fails_the_i686_check(self) -> None:
        observations = copy.deepcopy(self.observations)
        observations["MB-13/1"]["elf_class"] = "ELF64"
        self.assertTrue(roi.run_checks(observations)["MB-13/1"])

    def test_a_missing_vzeroupper_fails_the_avx_hygiene_check(self) -> None:
        observations = copy.deepcopy(self.observations)
        observations["SV-08/1"]["vzeroupper"] = 0
        self.assertTrue(roi.run_checks(observations)["SV-08/1"])


if __name__ == "__main__":
    unittest.main()
