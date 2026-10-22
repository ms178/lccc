"""The sharded benchmark lane must merge into exactly the single-corpus report.

`bench.yml` runs the registry in two shards so the lane's wall clock is the
slower shard, not the sum of every kernel.  That split is only safe if the
merged document is indistinguishable from the one a single job would publish:
same schema, same rows, same aggregates, one row per registered benchmark.
The failure modes are quiet -- a shard that silently dropped a kernel would
leave the report looking complete with a smaller corpus -- so they are pinned
here against synthetic reports, in the same style as the rest of the harness
contracts.
"""

from __future__ import annotations

import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from tests.benchmark import run_benchmarks as bench  # noqa: E402


def entry(name: str, *, lccc=1.0, gcc=2.0, lccc_ok=True, gcc_ok=True) -> dict:
    return {
        "name": name,
        "compile": {"lccc": {"ok": True}, "gcc": {"ok": True}},
        "runtime": {"lccc": {"wall": {"median": lccc}},
                    "gcc": {"wall": {"median": gcc}}},
        "ratios_to_gcc": {"lccc": {"median": lccc / gcc}},
        "ratios_from_lccc": {"gcc": {"median": gcc / lccc}},
        "correctness": {"lccc": {"matches_baseline": lccc_ok},
                        "gcc": {"matches_baseline": gcc_ok}},
        "warnings": [],
    }


def report(names: list[str], **overrides) -> dict:
    document = {
        "complete": True,
        "schema": bench.SCHEMA,
        "metadata": {"timestamp_utc": "2026-10-22T00:00:00Z", "cpu_models": ["test"],
                     "hypervisor_detected": False, "pinning": "none",
                     "pmu": {"available": False, "reason": "test"},
                     "git": {"revision": "deadbeef"}, "work_root": {}},
        "settings": {"opt": "-O2", "common_cflags": [], "reps": 1, "warmup": 0,
                     "skip_perf": False, "seed": 1},
        "compilers": [{"key": "lccc", "label": "LCCC", "executable": "/lccc",
                       "flags": [], "binary_sha256": "0" * 64},
                      {"key": "gcc", "label": "GCC", "executable": "/gcc",
                       "flags": [], "binary_sha256": "1" * 64}],
        "unavailable_compilers": {},
        "benchmarks": [entry(name) for name in names],
        "runner_warnings": [],
    }
    document.update(overrides)
    return document


class MergeReports(unittest.TestCase):
    def merge(self, *documents: dict) -> dict:
        with tempfile.TemporaryDirectory() as tmp:
            paths = []
            for index, document in enumerate(documents):
                path = Path(tmp) / f"shard-{index}.json"
                path.write_text(json.dumps(document))
                paths.append(path)
            return bench.merge_reports(paths)

    def test_union_preserves_rows_and_recomputes_aggregates(self):
        merged = self.merge(report(["a", "b"], complete=True),
                            report(["c"], complete=True))
        self.assertEqual([row["name"] for row in merged["benchmarks"]], ["a", "b", "c"])
        self.assertTrue(merged["complete"])
        self.assertEqual(merged["schema"], bench.SCHEMA)
        # The aggregate is over the UNION, not over the first shard.
        self.assertEqual(merged["aggregate_lccc_vs_gcc"]["n_correct_benchmarks"], 3)
        self.assertEqual(len(merged["shards"]), 2)

    def test_incomplete_shard_is_refused(self):
        partial = report(["a"], complete=False)
        with self.assertRaises(SystemExit) as caught:
            self.merge(report(["b"]), partial)
        self.assertIn("did not complete", str(caught.exception))

    def test_duplicate_benchmark_across_shards_is_refused(self):
        with self.assertRaises(SystemExit) as caught:
            self.merge(report(["a", "b"]), report(["b"]))
        self.assertIn("also appears in", str(caught.exception))

    def test_foreign_schema_is_refused(self):
        with self.assertRaises(SystemExit) as caught:
            self.merge(report(["a"]), report(["b"], schema="lccc-bench-v1"))
        self.assertIn("schema", str(caught.exception))

    def test_different_settings_are_refused(self):
        other = copy.deepcopy(report(["b"]))
        other["settings"] = dict(other["settings"], reps=9)
        with self.assertRaises(SystemExit) as caught:
            self.merge(report(["a"]), other)
        self.assertIn("settings differ", str(caught.exception))

    def test_different_compiler_identity_is_refused(self):
        other = copy.deepcopy(report(["b"]))
        other["compilers"] = list(other["compilers"])
        other["compilers"][0] = dict(other["compilers"][0], binary_sha256="2" * 64)
        with self.assertRaises(SystemExit) as caught:
            self.merge(report(["a"]), other)
        self.assertIn("compiler identities differ", str(caught.exception))

    def test_merged_report_renders_like_a_single_corpus(self):
        merged = self.merge(report(["a", "b"]), report(["c"]))
        markdown = bench.render_markdown(merged)
        for name in ("a", "b", "c"):
            self.assertIn(f"`{name}`", markdown)
        terminal = bench.render_terminal(merged)
        self.assertIn("n=3", terminal)


if __name__ == "__main__":
    unittest.main()
