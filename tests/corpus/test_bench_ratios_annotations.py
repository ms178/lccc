#!/usr/bin/env python3
"""Contract tests for `.github/scripts/ci_bench_ratios.py`.

The instrument exists so a benchmark shard's ratios are readable through the
checks API. What has to be pinned is that it never turns evidence into a gate
(a missing or malformed report must not fail a lane), that the published order
is worst-first, and that the annotation is opt-in so ordinary runs stay clean.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / ".github" / "scripts" / "ci_bench_ratios.py"


def run(args: list[str], env_extra: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    env.pop("LCCC_BENCH_RATIOS", None)
    if env_extra:
        env.update(env_extra)
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        cwd=REPO,
        check=False,
    )


def report(benchmarks: list[dict]) -> dict:
    return {
        "schema": "lccc-bench-v2",
        "settings": {"reps": 9, "warmup": 1},
        "metadata": {},
        "benchmarks": benchmarks,
        "aggregate_lccc_vs_gcc": {
            "n_correct_benchmarks": len(benchmarks),
            "geometric_mean_ratio": 1.25,
            "arithmetic_mean_ratio": 1.31,
            "best": {"benchmark": "fast_one", "ratio": 0.83},
            "worst": {"benchmark": "slow_one", "ratio": 2.10},
        },
    }


def entry(name: str, ratio: float, n: int = 9, warnings: list[str] | None = None) -> dict:
    return {
        "name": name,
        "ratios_to_gcc": {"lccc": {"median": ratio, "n": n}},
        "runtime": {"lccc": {"quality_warnings": []}},
        "warnings": warnings or [],
    }


class BenchRatiosTest(unittest.TestCase):
    def write(self, payload: dict) -> str:
        handle = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
        json.dump(payload, handle)
        handle.close()
        self.addCleanup(lambda p=handle.name: Path(p).unlink(missing_ok=True))
        return handle.name

    def test_missing_report_is_evidence_not_failure(self) -> None:
        result = run(["--report", "/nonexistent/bench-a.json", "--annotate"])
        self.assertEqual(result.returncode, 0)
        self.assertIn("no benchmark report", result.stdout)
        self.assertIn("::warning title=bench-ratios::", result.stdout)

    def test_malformed_report_is_evidence_not_failure(self) -> None:
        path = Path(self.write({}))
        path.write_text("{not json")
        result = run(["--report", str(path), "--annotate"])
        self.assertEqual(result.returncode, 0)
        self.assertIn("unreadable benchmark report", result.stdout)

    def test_foreign_schema_is_reported_not_trusted(self) -> None:
        payload = report([entry("a", 1.0)])
        payload["schema"] = "lccc-bench-v1"
        result = run(["--report", self.write(payload), "--annotate"])
        self.assertEqual(result.returncode, 0)
        self.assertIn("update this instrument", result.stdout)

    def test_worst_ratio_is_published_first(self) -> None:
        payload = report([entry("fast_one", 0.83), entry("slow_one", 2.10), entry("mid", 1.10)])
        result = run(["--report", self.write(payload), "--top", "3", "--annotate"])
        self.assertEqual(result.returncode, 0)
        lines = [line for line in result.stdout.splitlines()
                 if " n=" in line and not line.startswith("::")]
        self.assertEqual(len(lines), 3)
        self.assertIn("slow_one 2.100", lines[0])
        self.assertIn("mid 1.100", lines[1])
        self.assertIn("fast_one 0.830", lines[2])
        self.assertIn("geomean 1.250 over 3 kernels", result.stdout)

    def test_rows_are_capped_and_the_remainder_is_announced(self) -> None:
        payload = report([entry(f"k{i}", 1.0 + i / 100) for i in range(6)])
        payload["aggregate_lccc_vs_gcc"]["n_correct_benchmarks"] = 6
        result = run(["--report", self.write(payload), "--top", "2", "--annotate"])
        published = [line for line in result.stdout.splitlines()
                     if " n=" in line and not line.startswith("::")]
        self.assertEqual(len(published), 2)
        self.assertIn("... 4 more kernel(s)", result.stdout)

    def test_annotation_is_opt_in(self) -> None:
        payload = report([entry("a", 1.0)])
        quiet = run(["--report", self.write(payload)])
        self.assertEqual(quiet.returncode, 0)
        self.assertNotIn("::warning", quiet.stdout)
        self.assertIn("a 1.000", quiet.stdout)

    def test_quality_warnings_travel_with_the_row(self) -> None:
        payload = report([entry("noisy", 1.44, warnings=["median runtime below 20 ms"])])
        result = run(["--report", self.write(payload), "--annotate"])
        self.assertIn("noisy 1.440 n=9 [1 quality warning(s)]", result.stdout)

    def test_annotation_payload_is_a_single_escaped_line(self) -> None:
        payload = report([entry("a", 1.0), entry("b", 2.0)])
        result = run(["--report", self.write(payload), "--annotate"])
        annotations = [line for line in result.stdout.splitlines() if line.startswith("::warning")]
        self.assertEqual(len(annotations), 1)
        # A multi-line payload would be truncated by the runner; `%0A` is how
        # the table's newlines survive as one workflow command.
        self.assertNotIn("\n", annotations[0])
        self.assertIn("%0A", annotations[0])

    def test_nothing_to_publish_when_no_ratio_survived(self) -> None:
        payload = report([{"name": "broken", "ratios_to_gcc": {}, "warnings": ["compile failure"]}])
        result = run(["--report", self.write(payload), "--annotate"])
        self.assertEqual(result.returncode, 0)
        self.assertIn("no benchmark carries a paired ratio", result.stdout)


if __name__ == "__main__":
    unittest.main()
