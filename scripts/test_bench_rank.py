#!/usr/bin/env python3
"""A low-sample screening result must not become a confident regression claim."""
import json
import math
import subprocess
import sys
from pathlib import Path
import tempfile
import unittest

import bench_rank as rank


class QualityTests(unittest.TestCase):
    def test_low_sample_interval_is_not_a_verdict(self):
        for n, expected in [(3, None), (7, True)]:
            row = {"name": "kernel", "baseline_compiler": "gcc",
                   "ratios_to_gcc": {"lccc": {"n": n, "median": 2.0,
                                               "median_ci95_bootstrap": [1.9, 2.1]}},
                   "runtime": {"lccc": {"quality_warnings": ["fewer than seven timed paired rounds"] if n < 7 else []}},
                   "correctness": {"lccc": {"matches_baseline": True}, "gcc": {"matches_baseline": True}}}
            with tempfile.TemporaryDirectory() as td:
                path = Path(td) / "report.json"
                path.write_text(json.dumps({"benchmarks": [row]}))
                rows, document = rank.load_rows(path)
            self.assertIs(rows[0]["significant"], expected)
            self.assertTrue(rows[0]["bootstrap_excludes_parity"])
            groups = rank.classify(rows, 0.01)
            self.assertEqual(len(groups["unknown" if n < 7 else "regressed"]), 1)
            if n < 7:
                self.assertIn("WARNING", rank.render_table(rows, show_size=False, reference="gcc"))
                self.assertIn("fewer than seven", rank.render_markdown(rows, document, groups, 0.01))


class MarkdownTests(unittest.TestCase):
    def test_full_aggregate_with_limited_table_and_invalid_pairs(self):
        rows = [dict(name=name, ratio=ratio, correct=correct, significant=True)
                for name, ratio, correct in [("slow", 2.0, True), ("fast", 0.5, True),
                                             ("failed", 100.0, False), ("invalid", math.nan, True)]]
        out = rank.render_markdown(rows, {"aggregate_lccc_vs_gcc": {"geometric_mean_ratio": 99}},
                                   rank.classify(rows, 0.01), 0.01, limit=1)
        self.assertIn("geometric mean ratio: **1.0000**", out)
        self.assertIn("arithmetic mean ratio: **1.2500**", out)
        self.assertIn("correct comparable pairs: **2**", out)
        self.assertIn("`slow`", out)
        self.assertNotIn("`fast`", out)
        self.assertNotIn("nan", out.lower())

    def test_nonfinite_producer_samples_are_not_numbers(self):
        for value in [math.nan, math.inf, -math.inf, True, "1.0"]:
            self.assertIsNone(rank._median({"median": value}))
        self.assertEqual(rank.geometric_mean([2.0, 0.5, math.nan, math.inf]), 1.0)

    def test_markdown_cli_honors_worst_and_all(self):
        rows = [{"name": name, "ratios_to_gcc": {"lccc": {"median": ratio}},
                 "correctness": {"lccc": {"matches_baseline": True},
                                 "gcc": {"matches_baseline": True}}}
                for name, ratio in [("slow", 2.0), ("fast", 0.5)]]
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "report.json"
            path.write_text(json.dumps({"benchmarks": rows}))
            cmd = [sys.executable, str(Path(rank.__file__)), str(path), "--markdown", "--worst", "1"]
            limited = subprocess.run(cmd, check=True, capture_output=True, text=True).stdout
            complete = subprocess.run(cmd + ["--all"], check=True, capture_output=True, text=True).stdout
        self.assertIn("geometric mean ratio: **1.0000**", limited)
        self.assertIn("`slow`", limited)
        self.assertNotIn("`fast`", limited)
        self.assertIn("`fast`", complete)


if __name__ == "__main__":
    unittest.main()


class LegacySchemaTests(unittest.TestCase):
    """The 2026-08/09 producer wrote flat medians, not the nested schema."""

    REPORT = (Path(__file__).resolve().parents[1] / "engineering" / "evidence"
              / "benchmarks" / "2026-09-27-s13-685da054" / "results.json")

    def test_checked_in_legacy_report_is_ranked_not_written_off_as_failed(self):
        rows, document = rank.load_rows(self.REPORT)
        self.assertGreaterEqual(len(rows), 8)
        self.assertEqual(document["benchmarks"][0]["name"], "arith_loop")
        self.assertTrue(all(row["correct"] for row in rows))
        self.assertEqual(len(rank.classify(rows, 0.01)["unknown"]), 0)
        table = rank.render_table(rows, show_size=False, reference="gcc")
        self.assertNotIn("INCORRECT/FAILED", table)
        self.assertIn("nbody", table)
        # The legacy record carries a paired interval and a sample count, so the
        # significance policy must still apply rather than being bypassed.
        nbody = next(row for row in rows if row["name"] == "nbody")
        self.assertIs(nbody["significant"], True)
        self.assertAlmostEqual(nbody["ratio"], 1.4140922617686313, places=9)

    def test_low_sample_legacy_row_is_not_a_verdict(self):
        row = {"name": "kernel", "correct": True, "lccc_median_s": 2.0,
               "gcc_median_s": 1.0, "ratio": 2.0, "ci95": [1.9, 2.1], "n": 3}
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "report.json"
            path.write_text(json.dumps({"benchmarks": [row]}))
            rows, _ = rank.load_rows(path)
        self.assertIsNone(rows[0]["significant"])
        self.assertIn(rows[0], rank.classify(rows, 0.01)["unknown"])

    def test_unreadable_schema_is_an_error_not_a_failed_workload(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "report.json"
            path.write_text(json.dumps({"benchmarks": [{"name": "mystery", "value": 1}]}))
            with self.assertRaises(ValueError) as raised:
                rank.load_rows(path)
        self.assertIn("no known evidence schema", str(raised.exception))
        self.assertIn("mystery", str(raised.exception))
