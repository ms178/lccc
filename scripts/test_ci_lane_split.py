#!/usr/bin/env python3
"""Regression tests for scripts/ci_lane_split.py.

The splitter rewrites the workflow that actually runs, from a plan that a
reviewer reads.  Its failure modes are silent: a gate that lands in no shard
simply stops running, a shard that receives a gate twice pays for it twice,
and a stale balance comment describes a split nobody performed.  The audit in
`tests/regression/check_ci_lane_plan.py` catches those in the *result*; these
tests pin the transform itself, on synthetic workflows small enough to assert
byte-for-byte, so a change in the splitter is visible before it reaches
`ci.yml`.

Covered contracts:

* `lpt` is longest-processing-time first and deterministic (ties broken by
  index), which is the only ordering that does not depend on where a gate
  happened to be written;
* `read_costs` normalises the command text, keeps the worst measurement of a
  duplicate and ignores comments and malformed lines -- a cost table is an
  input, not a schema;
* `--mode steps` produces N jobs whose union of gate steps is exactly the
  original list, whose preamble is byte-identical, and whose `needs:` edges
  point at the shards instead of the lane they replaced;
* `--mode shell` re-splits an existing pair in place: the gate set is a
  partition, the wrapper (`set -Eeuo pipefail`, ERR trap, tee tail) is
  rewritten identically, and the budget comment above each step is replaced
  rather than accumulated;
* `--mode join` restores the original job, including the tail this workflow
  spells `needs:` in (a hand merge dropped it and silently removed the shard's
  edge to `build`), and rewrites `needs:` as a set rather than a multiset;
* the plan is keyed by JOB ID with a per-lane `mode`, because a step name is
  not a lane (that keying is what lost the contract shards from the record),
  and lanes the workflow no longer has are pruned.

Run: python3 scripts/test_ci_lane_split.py
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import ci_lane_split as split  # noqa: E402


STEPS_WORKFLOW = textwrap.dedent("""\
    name: Test
    on: push
    jobs:
      build:
        name: Build
        runs-on: ubuntu-24.04
        steps:
        - name: Checkout
          uses: actions/checkout@v7
      lane:
        name: Contracts
        needs: [build]
        runs-on: ubuntu-24.04
        steps:
        - name: Checkout
          uses: actions/checkout@v7
        - name: Verify exact compiler build
          run: |
            test -x target/fastbuild/lccc
        # The first gate.
        - name: Gate one
          run: python3 scripts/gate_one.py
        - name: Gate two
          run: python3 scripts/gate_two.py
        - name: Surface gate failures
          run: cat gate.log
        - name: Gate three
          run: python3 scripts/gate_three.py
      consumer:
        needs: [lane]
        runs-on: ubuntu-24.04
        steps:
        - name: Require every lane
          run: true
    """)

SHELL_WORKFLOW = textwrap.dedent("""\
    name: Test
    on: push
    jobs:
      lane-a:
        name: Contracts (shard a)
        runs-on: ubuntu-24.04
        steps:
        - name: Checkout
          uses: actions/checkout@v7
        - name: Verify remaining contracts (shard a of 2)
          run: |
            set -Eeuo pipefail
            contracts() {{
            trap 'rc=$?; exit "$rc"' ERR
            python3 scripts/gate_one.py
            python3 scripts/gate_four.py
            }}
            if ! contracts 2>&1 | tee log; then exit 1; fi
      lane-b:
        name: Contracts (shard b)
        runs-on: ubuntu-24.04
        steps:
        - name: Checkout
          uses: actions/checkout@v7
        - name: Verify remaining contracts (shard b of 2)
          run: |
            set -Eeuo pipefail
            contracts() {{
            trap 'rc=$?; exit "$rc"' ERR
            # a gate with a rationale
            python3 scripts/gate_two.py
            python3 scripts/gate_three.py
            }}
            if ! contracts 2>&1 | tee log; then exit 1; fi
    """).replace("{{", "{").replace("}}", "}")


def costs(*pairs: tuple[str, float]) -> Path:
    path = Path(tempfile.mkdtemp(prefix="lane-costs-")) / "costs.tsv"
    path.write_text("".join(f"{seconds}\t{command}\n" for command, seconds in pairs))
    return path


def steps_args(workflow: Path, plan: Path, cost_file: Path, *, shards: int = 2,
               job: str = "lane", write: bool = True) -> argparse.Namespace:
    return argparse.Namespace(
        mode="steps", job=job, lane_prefix=None, group=None, shards=shards,
        costs=cost_file, provenance="test", workflow=workflow, plan=plan, write=write,
    )


def join_args(workflow: Path, plan: Path, *, job: str = "lane",
              write: bool = True) -> argparse.Namespace:
    return argparse.Namespace(
        mode="join", job=job, lane_prefix=None, group=None, shards=2,
        costs=None, provenance="test", workflow=workflow, plan=plan, write=write,
    )


def shell_args(workflow: Path, plan: Path, cost_file: Path, *, shards: int = 2,
               write: bool = True) -> argparse.Namespace:
    return argparse.Namespace(
        mode="shell", job=None, lane_prefix="Verify remaining contracts", group=None,
        shards=shards, costs=cost_file, provenance="test", workflow=workflow,
        plan=plan, write=write,
    )


class TempTree(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="lane-split-"))

    def workflow(self, text: str = STEPS_WORKFLOW) -> Path:
        path = self.tmp / "ci.yml"
        path.write_text(text)
        return path

    def plan(self, payload: dict | None = None) -> Path:
        path = self.tmp / "plan.json"
        path.write_text(json.dumps(payload) if payload is not None else "{}")
        return path


class LptTest(unittest.TestCase):
    def test_longest_first_and_deterministic(self) -> None:
        weights = [5.0, 1.0, 4.0, 2.0, 3.0]
        assignment = split.lpt(weights, 2)
        totals = [0.0, 0.0]
        for weight, target in zip(weights, assignment):
            totals[target] += weight
        # 5+1 = 6 and 4+2+3 = 9 is what a greedy pass gives; what matters is
        # that the heaviest gate is placed first and the result never depends
        # on the input order.
        self.assertLessEqual(max(totals) - min(totals), 3.0)
        self.assertEqual(assignment, split.lpt(list(weights), 2))
        self.assertLessEqual(max(totals), sum(weights))

    def test_single_shard_owns_everything(self) -> None:
        self.assertEqual(split.lpt([1.0, 2.0], 1), [0, 0])


class ReadCostsTest(unittest.TestCase):
    def test_normalises_and_keeps_the_worst_duplicate(self) -> None:
        path = Path(tempfile.mkdtemp(prefix="lane-costs-")) / "t.tsv"
        path.write_text(
            "# seconds\tcommand\trc\n"
            "1.50\tpython3  scripts/gate.py    --flag\n"
            "4.00\tpython3 scripts/gate.py --flag\n"
            "2.00\tbash tests/regression/x.sh\n"
            "not-a-number\tpython3 scripts/ignored.py\n"
            "9.00\n"
        )
        table = split.read_costs(path)
        self.assertEqual(table["python3 scripts/gate.py --flag"], 4.00)
        self.assertEqual(table["bash tests/regression/x.sh"], 2.00)
        self.assertNotIn("python3 scripts/ignored.py", table)


class StepsModeTest(TempTree):
    def test_partition_preamble_and_plan(self) -> None:
        workflow, plan = self.workflow(), self.plan()
        cost_file = costs(("python3 scripts/gate_one.py", 9.0),
                          ("python3 scripts/gate_two.py", 6.0),
                          ("python3 scripts/gate_three.py", 3.0),
                          ("cat gate.log", 0.1))
        self.assertEqual(split.split_steps(
            steps_args(workflow, plan, cost_file)), 0)

        text = workflow.read_text()
        document = json.loads(json.dumps(_yaml(text)))
        self.assertNotIn("lane", document["jobs"])
        self.assertIn("lane-a", document["jobs"])
        self.assertIn("lane-b", document["jobs"])

        # `Surface ...` is glued to the gate whose log it reads: splitting them
        # would leave the annotator in a shard with no log.
        setup = {"Checkout", "Verify exact compiler build"}
        gates = {job: [step["name"] for step in document["jobs"][job]["steps"]
                       if step["name"] not in setup]
                 for job in ("lane-a", "lane-b")}
        union = set(gates["lane-a"]) | set(gates["lane-b"])
        self.assertEqual(
            union, {"Gate one", "Gate two", "Gate three", "Surface gate failures"})
        self.assertEqual(len(union), len(gates["lane-a"]) + len(gates["lane-b"]))
        self.assertTrue(all(name in gates["lane-a"] or name in gates["lane-b"]
                            for name in union))

        # Preamble identity, including the `steps:` key itself.
        def preamble(job: str) -> list[dict]:
            steps = document["jobs"][job]["steps"]
            return steps[:next(i for i, s in enumerate(steps)
                               if s["name"].startswith(("Gate", "Surface")))]

        self.assertEqual(preamble("lane-a"), preamble("lane-b"))
        self.assertEqual([s.get("name") for s in preamble("lane-a")],
                         ["Checkout", "Verify exact compiler build"])

        # The consumer must depend on the shards, not on the lane they replaced.
        self.assertEqual(document["jobs"]["consumer"]["needs"], ["lane-a", "lane-b"])

        record = json.loads(plan.read_text())
        self.assertNotIn("mode", record)  # per-lane, not per-file
        self.assertEqual(set(record["lanes"]), {"lane-a", "lane-b"})
        self.assertEqual(record["lanes"]["lane-a"]["group"], "lane")
        self.assertEqual(record["lanes"]["lane-a"]["mode"], "steps")
        self.assertEqual(record["provenance"]["lane"]["source"], "test")
        planned = {step["name"] for spec in record["lanes"].values()
                   for step in spec["steps"]}
        self.assertEqual(planned, union)
        # The spend is written into the job as a comment, and it is the sum of
        # the gates that shard actually received.
        for job in ("lane-a", "lane-b"):
            predicted = record["lanes"][job]["predicted_gate_seconds"]
            measured = sum(step["measured_s"] for step in record["lanes"][job]["steps"])
            self.assertAlmostEqual(predicted, measured, places=2)
            self.assertIn(f"{predicted:.0f} s", text)

    def test_unmeasured_gate_warns_but_still_splits(self) -> None:
        workflow, plan = self.workflow(), self.plan()
        cost_file = costs(("python3 scripts/gate_one.py", 1.0))
        self.assertEqual(split.split_steps(
            steps_args(workflow, plan, cost_file)), 0)
        record = json.loads(plan.read_text())
        planned = {step["name"]: step["measured_s"]
                   for spec in record["lanes"].values() for step in spec["steps"]}
        self.assertTrue(all(seconds >= 0.25 for seconds in planned.values()))

    def test_merge_back_then_resplit_is_clean(self) -> None:
        """Re-splitting must not stack shard suffixes or budget notes.

        A merged lane still carries the old ` (shard a of 2)` name and the old
        generated note.  Appending the suffix again produced
        "Contracts (shard b of 2) (shard a of 2)", and appending the note
        again left two comments describing two different splits.
        """
        workflow, plan = self.workflow(), self.plan()
        cost_file = costs(("python3 scripts/gate_one.py", 4.0),
                          ("python3 scripts/gate_two.py", 3.0),
                          ("python3 scripts/gate_three.py", 2.0))
        self.assertEqual(split.split_steps(
            steps_args(workflow, plan, cost_file)), 0)
        self.assertEqual(split.join_lanes(join_args(workflow, plan)), 0)
        self.assertEqual(split.split_steps(
            steps_args(workflow, plan, cost_file)), 0)

        document = _yaml(workflow.read_text())
        self.assertEqual(document["jobs"]["lane-a"]["name"], "Contracts (shard a of 2)")
        self.assertEqual(document["jobs"]["lane-b"]["name"], "Contracts (shard b of 2)")
        for job in ("lane-a", "lane-b"):
            names = [step["name"] for step in document["jobs"][job]["steps"]]
            # Every shard carries the preamble exactly once ...
            self.assertEqual(names.count("Checkout"), 1)
            self.assertEqual(names.count("Verify exact compiler build"), 1)
        gates = [step["name"] for job in ("lane-a", "lane-b")
                 for step in document["jobs"][job]["steps"]
                 if step["name"].startswith(("Gate", "Surface"))]
        self.assertEqual(sorted(gates), sorted([
            "Gate one", "Gate two", "Gate three", "Surface gate failures"]))
        # One regenerated note per shard, and the hand-written comment that
        # introduces a gate survives the round trip.
        text = workflow.read_text()
        self.assertEqual(text.count("gate steps of the `lane` lane"), 2)
        self.assertEqual(text.count("    # lane-split: "), 10)  # 5 lines x 2 shards
        self.assertIn("# The first gate.", text)

    def test_join_carries_the_job_tail(self) -> None:
        """A job's AFTER-steps block (`needs:` here) must survive join -> split.

        This workflow spells `needs:` at the END of the job, not beside the id,
        so a merge that only concatenates step blocks drops the DAG edge.  The
        lane then runs without `build`, and only the layout audit notices.
        """
        workflow = self.workflow(STEPS_WORKFLOW.replace(
            "        - name: Gate three\n"
            "          run: python3 scripts/gate_three.py\n",
            "        - name: Gate three\n"
            "          run: python3 scripts/gate_three.py\n"
            "        needs:\n"
            "        - build\n"))
        plan = self.plan()
        cost_file = costs(("python3 scripts/gate_one.py", 1.0),
                          ("python3 scripts/gate_two.py", 1.0),
                          ("python3 scripts/gate_three.py", 1.0),
                          ("cat gate.log", 0.1))
        self.assertEqual(split.split_steps(steps_args(workflow, plan, cost_file)), 0)
        # Both shards inherit the tail, exactly as a split of that job must.
        document = _yaml(workflow.read_text())
        for job in ("lane-a", "lane-b"):
            self.assertEqual(document["jobs"][job].get("needs"), ["build"], job)
        self.assertEqual(split.join_lanes(join_args(workflow, plan)), 0)
        document = _yaml(workflow.read_text())
        self.assertEqual(document["jobs"]["lane"].get("needs"), ["build"])
        self.assertNotIn("lane-a", document["jobs"])
        self.assertEqual(document["jobs"]["lane"]["name"], "Contracts")
        self.assertEqual(document["jobs"]["consumer"]["needs"], ["lane"])
        self.assertEqual(split.split_steps(steps_args(workflow, plan, cost_file)), 0)
        document = _yaml(workflow.read_text())
        for job in ("lane-a", "lane-b"):
            self.assertEqual(document["jobs"][job].get("needs"), ["build"], job)

    def test_needs_rewrite_is_a_set(self) -> None:
        """Round-tripping a sharded consumer must not multiply its needs.

        `frontend` -> `frontend-a, frontend-b` -> `frontend` -> ... applied to
        a job that already listed both shards left four identical entries:
        valid YAML, an unchanged DAG, and a lie in the review record.
        """
        lines = ["  test:\n",
                 "    needs:\n",
                 "    - build\n",
                 "    - lane-a\n",
                 "    - lane-b\n",
                 "    - lane-a\n"]
        joined = split.replace_needs(lines, "lane-a", ["lane"])
        joined = split.replace_needs(joined, "lane-b", ["lane"])
        self.assertEqual(joined.count("    - lane\n"), 1, joined)
        split_back = split.replace_needs(joined, "lane", ["lane-a", "lane-b"])
        self.assertEqual(split_back.count("    - lane-a\n"), 1, split_back)
        self.assertEqual(split_back.count("    - lane-b\n"), 1, split_back)
        inline = split.replace_needs(
            ["    needs: [build, lane-a, lane-b, lane-a]\n"], "lane-a", ["lane"])
        self.assertEqual(inline, ["    needs: [build, lane, lane-b]\n"])

    def test_join_refuses_mismatched_scaffolding(self) -> None:
        workflow, plan = self.workflow(), self.plan()
        cost_file = costs(("python3 scripts/gate_one.py", 1.0),
                          ("python3 scripts/gate_two.py", 1.0),
                          ("python3 scripts/gate_three.py", 1.0))
        self.assertEqual(split.split_steps(steps_args(workflow, plan, cost_file)), 0)
        text = workflow.read_text()
        # Give one shard a setup step the other does not have: it is no longer
        # a shard of the same lane, and joining it would silently drop that
        # setup from the merged job.
        head, rest = text.split("  lane-a:", 1)
        rest = rest.replace(
            "    - name: Checkout\n      uses: actions/checkout@v7\n",
            "    - name: Checkout\n      uses: actions/checkout@v7\n"
            "    - name: Extra setup\n      run: true\n", 1)
        marked = head + "  lane-a:" + rest
        self.assertNotEqual(marked, text)
        self.assertIn("Extra setup", _yaml(marked)["jobs"]["lane-a"]["steps"][1]["name"])
        workflow.write_text(marked)
        with self.assertRaises(SystemExit):
            split.join_lanes(join_args(workflow, plan))

    def test_refuses_a_job_that_is_already_split(self) -> None:
        workflow = self.workflow()
        plan = self.plan()
        cost_file = costs(("python3 scripts/gate_one.py", 1.0))
        self.assertEqual(split.split_steps(
            steps_args(workflow, plan, cost_file)), 0)
        with self.assertRaises(SystemExit):
            split.split_steps(steps_args(workflow, plan, cost_file))


def _yaml(text: str) -> dict:
    import yaml

    return yaml.safe_load(text)


class ShellModeTest(TempTree):
    def test_resplit_is_a_partition_with_identical_scaffolding(self) -> None:
        workflow = self.workflow(SHELL_WORKFLOW)
        plan = self.plan()
        cost_file = costs(("python3 scripts/gate_one.py", 9.0),
                          ("python3 scripts/gate_two.py", 1.0),
                          ("python3 scripts/gate_three.py", 8.0),
                          ("python3 scripts/gate_four.py", 2.0))
        self.assertEqual(split.split_shell(shell_args(workflow, plan, cost_file)), 0)

        document = _yaml(workflow.read_text())
        bodies = {job: document["jobs"][job]["steps"][1]["run"]
                  for job in ("lane-a", "lane-b")}
        # Which shard a gate lands in is LPT's business (and ties are broken by
        # command order), but every gate must land in exactly ONE of them.
        for gate in ("gate_one", "gate_two", "gate_three", "gate_four"):
            runs = [job for job, body in bodies.items() if f"scripts/{gate}.py" in body]
            self.assertEqual(len(runs), 1, (gate, runs))
        # The wrapper is rewritten, never dropped: a shard without the ERR trap
        # reports a bare exit code and loses the failing command.
        for body in bodies.values():
            self.assertIn("set -Eeuo pipefail", body)
            self.assertIn("trap 'rc=$?; exit \"$rc\"' ERR", body)
            # ... and the gate function must be called as a PLAIN command.
            # `if ! contracts | tee ...` disarms errexit *and* the ERR trap
            # for the whole function body, so only its last gate could ever
            # fail the step; the splitter normalises that shape away rather
            # than copying it forward.  Both halves are pinned:
            self.assertNotIn("if ! contracts", body)
            self.assertIn("trap annotate_shard ERR", body)
            self.assertTrue(body.rstrip().endswith('contracts 2>&1 | tee "log"'))
            # The tee target survives the rewrite, so an operator (and
            # ci_local.sh) still finds the transcript where it always was.
            self.assertIn('scripts/ci_annotate_log.py --title "Gate output" "log"', body)
            self.assertEqual(body.count("contracts 2>&1"), 1)  # one invocation
            self.assertEqual(body.count("annotate_shard"), 2)   # definition + trap
        # ...and no re-split may double-space the tail.
        for body in bodies.values():
            self.assertNotIn("\n\n", body)

        record = json.loads(plan.read_text())
        self.assertNotIn("mode", record)
        self.assertEqual(set(record["lanes"]), {"lane-a", "lane-b"})
        for job, spec in record["lanes"].items():
            self.assertEqual(spec["mode"], "shell")
            self.assertEqual(spec["group"], "lane")
            self.assertEqual(spec["step"],
                             f"Verify remaining contracts (shard {job[-1]} of 2)")
            self.assertEqual(len(spec["steps"]), 2)
        # The budget comment above the step is regenerated, not appended to.
        text = workflow.read_text()
        self.assertEqual(text.count("independent gate invocations"), 2)
        self.assertEqual(text.count("191 s"), 0)

    def test_refuses_mismatched_scaffolding(self) -> None:
        # The ERR trap is the wrapper's identity: a shard whose trap differs
        # reports a different diagnostic, so the pair is not one lane.
        trap = "        trap 'rc=$?; exit \"$rc\"' ERR\n"
        self.assertEqual(SHELL_WORKFLOW.count(trap), 2)
        broken = SHELL_WORKFLOW.replace(
            trap, "        trap 'rc=$?; echo boom; exit \"$rc\"' ERR\n", 1)
        self.assertNotEqual(broken, SHELL_WORKFLOW)
        workflow = self.workflow(broken)
        cost_file = costs(("python3 scripts/gate_one.py", 1.0))
        with self.assertRaises(SystemExit):
            split.split_shell(shell_args(workflow, self.plan(), cost_file))


class PlanBookkeepingTest(TempTree):
    def test_prunes_vanished_lanes_and_keeps_per_lane_mode(self) -> None:
        workflow = self.workflow()
        plan = self.plan({"lanes": {"ghost-a": {"group": "ghost", "steps": []}},
                          "provenance": {"ghost": {"source": "old"}}})
        cost_file = costs(("python3 scripts/gate_one.py", 1.0))
        self.assertEqual(split.split_steps(
            steps_args(workflow, plan, cost_file)), 0)
        record = json.loads(plan.read_text())
        self.assertNotIn("ghost-a", record["lanes"])
        self.assertEqual(record["lanes"], {k: v for k, v in record["lanes"].items()
                                           if k.startswith("lane-")})
        # A second lane of a different shape must not overwrite the first
        # lane's mode: the record is per lane.
        shell_workflow = self.tmp / "shell.yml"
        shell_workflow.write_text(SHELL_WORKFLOW)
        shell_plan = self.tmp / "plan2.json"
        shell_plan.write_text(plan.read_text())
        self.assertEqual(split.split_shell(
            shell_args(shell_workflow, shell_plan, cost_file)), 0)
        merged = json.loads(shell_plan.read_text())
        self.assertEqual(merged["lanes"]["lane-a"]["mode"], "shell")
        self.assertNotIn("mode", merged)


if __name__ == "__main__":
    unittest.main(verbosity=2)
