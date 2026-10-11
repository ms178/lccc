#!/usr/bin/env python3
"""A low-sample screening result must not become a confident regression claim."""
import json
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


if __name__ == "__main__":
    unittest.main()
