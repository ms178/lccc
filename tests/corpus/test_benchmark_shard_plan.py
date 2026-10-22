"""The sharded benchmark lane must be a partition of the registry.

`bench.yml` runs the corpus in shards so its wall clock is the slowest shard
instead of the sum of every kernel.  Two silent failures follow from that:

* a kernel that appears in no shard is simply never measured -- the merged
  report still looks complete, because every shard finished;
* a kernel in two shards is measured twice, which costs runner time and (worse)
  makes a merged report ambiguous, since `merge_reports` refuses duplicates and
  the pipeline would only fail at the very end.

The registry (`tests.benchmark.run_benchmarks.BENCHMARKS`) is the source of
truth for what the "complete" corpus is, so the partition is checked against
it directly rather than against a second list that could drift.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from tests.benchmark.run_benchmarks import BENCHMARKS  # noqa: E402

CI = ROOT / ".github/workflows/ci.yml"
BENCH = "./.github/workflows/bench.yml"


def shard_inputs(document: dict) -> list[dict]:
    """`with:` blocks of every job that calls the benchmark workflow."""
    return [job["with"] for job in document["jobs"].values()
            if job.get("uses") == BENCH]


class BenchmarkShardPlan(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.document = yaml.safe_load(CI.read_text())
        cls.shards = shard_inputs(cls.document)
        cls.registry = [benchmark.name for benchmark in BENCHMARKS]

    def test_the_lane_is_sharded(self) -> None:
        self.assertGreaterEqual(len(self.shards), 2,
                                "the benchmark lane is no longer sharded")

    def test_shards_partition_the_registry(self) -> None:
        owned: dict[str, str] = {}
        for shard in self.shards:
            names = [name.strip() for name in shard["only"].split(",") if name.strip()]
            self.assertTrue(names, f"shard {shard['shard']} owns no kernel")
            for name in names:
                self.assertNotIn(name, owned,
                                 f"{name!r} is in shards {owned.get(name)} and {shard['shard']}")
                owned[name] = shard["shard"]
        self.assertEqual(sorted(owned), sorted(self.registry),
                         "the shards must cover exactly the registry")
        self.assertEqual(len(set(owned.values())), len(self.shards))

    def test_every_shard_reports_to_a_distinct_artifact(self) -> None:
        artifacts = [shard["artifact"] for shard in self.shards]
        self.assertEqual(len(set(artifacts)), len(artifacts))
        reports = [shard["report"] for shard in self.shards]
        self.assertEqual(len(set(reports)), len(reports))

    def test_exactly_one_shard_runs_the_static_gates(self) -> None:
        # The fib/codegen gates are cheap but not free, and they measure the
        # same six golden workloads regardless of shard: running them in every
        # shard is duplicated work with no extra coverage.
        flag = [bool(shard.get("gate_steps")) for shard in self.shards]
        self.assertEqual(sum(flag), 1, f"gate_steps set in {sum(flag)} shards")

    def test_the_merge_job_depends_on_every_shard(self) -> None:
        merge = next((job for job in self.document["jobs"].values()
                      if (job.get("name") or "").startswith("Complete benchmark corpus")),
                     None)
        self.assertIsNotNone(merge, "no merge job publishes the complete corpus")
        needs = set(merge.get("needs") or [])
        self.assertEqual(len(needs), len(self.shards),
                         f"merge needs {sorted(needs)} but there are {len(self.shards)} shards")
        self.assertIn("--merge", "\n".join(step.get("run") or "" for step in merge["steps"]))
        publish = [step for step in merge["steps"]
                   if (step.get("with") or {}).get("name") == "benchmark-results"]
        self.assertTrue(publish, "the merged report must publish as `benchmark-results`")


if __name__ == "__main__":
    unittest.main()
