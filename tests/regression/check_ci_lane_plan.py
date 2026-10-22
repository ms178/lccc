#!/usr/bin/env python3
"""The sharded lanes must match the partition they were split from.

`scripts/ci_lane_split.py` divides a lane's gates into balanced shards and
records the assignment in `.github/ci-lane-plan.json`.  The workflow, not the
plan, is what runs, so this gate is what keeps them honest -- and every failure
mode here is SILENT in a green run:

* a gate listed in both shards runs twice and costs twice;
* a gate that fell out of both shards is lost coverage, and only an audit of
  the union notices;
* a shard that lost a preamble step (the toolchain cache, the oracle probe)
  quietly pays the apt transaction the pipeline removed, or runs without the
  compiler the artifact step published;
* a shard that drifted from the measured balance stops buying the parallelism
  it was created for.

So the audit is structural and cheap: the plan names the gates of each shard,
and this gate proves the workflow still contains exactly that set, once, with a
byte-identical preamble and a balance within tolerance.

``--self-test`` exercises each rule against synthetic fixtures, including the
near-misses (gate moved, gate copied, preamble edited, balance wrecked),
because a checker that cannot fail is decoration.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/ci.yml"
PLAN = ROOT / ".github/ci-lane-plan.json"
BALANCE_TOLERANCE = 0.05


def step_name(step: dict) -> str:
    return step.get("name") or step.get("uses") or "<unnamed>"


def plan_groups(plan: dict) -> dict[str, list[str]]:
    groups: dict[str, list[str]] = {}
    for lane, spec in plan.get("lanes", {}).items():
        groups.setdefault(spec.get("group", lane.rsplit("-", 1)[0]), []).append(lane)
    return {group: sorted(lanes) for group, lanes in sorted(groups.items())}


def group_mode(group: str, lanes: list[str], plan: dict) -> str:
    """How the gates of one group are spelled: `steps` or `shell`.

    The mode belongs to the LANE, not to the file.  A plan that holds both
    shapes -- `frontend-a` (one step per gate) beside `backend-contracts-a`
    (one run body of commands) -- cannot be read with a single grammar: the
    step grammar sees the contract shard's one step and reports every planned
    command as missing, and the shell grammar tears a step list apart looking
    for an ERR trap.  The old file-level `mode` also came from `setdefault`,
    so it recorded whichever group happened to be split first.
    """
    for lane in lanes:
        mode = plan["lanes"][lane].get("mode")
        if mode:
            return mode
    provenance = (plan.get("provenance") or {}).get(group) or {}
    return provenance.get("mode") or plan.get("mode", "steps")


def unplanned_shards(jobs: dict, plan: dict) -> list[str]:
    """Sharded jobs that run inline steps and are not in the plan.

    These are the jobs `scripts/ci_lane_split.py` creates: a group of sibling
    job ids differing only in a one-letter suffix, each carrying its own
    `steps:`.  Reusable-workflow shards (`uses:`) are excluded -- the benchmark
    shards are recorded in `ci.yml`'s kernel lists and audited by
    `tests/corpus/test_benchmark_shard_plan.py` -- and so is any single job
    that merely ends in `-a`.

    This is the rule that would have caught the real bug: the contract shards
    left the plan when the next split of a different lane pruned their
    step-name keys as stale, and nothing noticed that two hosted jobs were
    running 139 gates with no recorded partition.
    """
    planned = set(plan.get("lanes", {}))
    by_stem: dict[str, list[str]] = {}
    inline = {job for job, spec in jobs.items() if spec.get("steps")}
    for job in inline:
        stem, _, suffix = job.rpartition("-")
        if stem and len(suffix) == 1 and suffix.isalpha():
            by_stem.setdefault(stem, []).append(job)
    missing: list[str] = []
    for stem, lanes in sorted(by_stem.items()):
        if len(lanes) > 1:
            missing.extend(f"{lane} (shards of {stem})" for lane in sorted(lanes)
                           if lane not in planned)
    return missing


def normalise(text: str) -> str:
    return " ".join(text.split())


def shard_gates(step: dict, planned: set[str], mode: str) -> list[str]:
    """The gates a shard step executes, named the way the plan names them.

    In `steps` mode each gate is its own step, so the name is the step name.
    In `shell` mode the gates live inside one run body -- and the *last* shard
    step also owns the tail of the body, so the body is split at the ERR trap
    and the closing brace, exactly as the splitter wrote it.
    """
    if mode == "steps":
        return [step_name(step)]
    body = step.get("run") or ""
    lines = body.splitlines()
    trap = next((i for i, line in enumerate(lines) if re.match(r"^\s*trap\b.*\bERR\b", line)),
                None)
    if trap is None:
        return []
    close = next((i for i in range(trap + 1, len(lines)) if lines[i].strip() == "}"),
                 len(lines))
    units: list[str] = []
    buffer: list[str] = []
    for line in lines[trap + 1:close]:
        stripped = line.strip()
        if not buffer and (not stripped or stripped.startswith("#")):
            continue
        buffer.append(line)
        if not line.rstrip().endswith("\\"):
            units.append(normalise(" ".join(buffer)))
            buffer = []
    if buffer:
        units.append(normalise(" ".join(buffer)))
    return units


TRAP_LINE = re.compile(r"^\s*trap\b.*\bERR\b", re.MULTILINE)


def gate_start(step_list: list[dict], planned: set[str], mode: str) -> int:
    """Index of the first gate-carrying step; everything before it is preamble."""
    if mode == "shell":
        return next((i for i, step in enumerate(step_list)
                     if TRAP_LINE.search(step.get("run") or "")), len(step_list))
    return next((i for i, step in enumerate(step_list)
                 if step_name(step) in planned), len(step_list))


def preamble(step_list: list[dict], planned: set[str], mode: str) -> list[str]:
    cut = gate_start(step_list, planned, mode)
    return [json.dumps(step, sort_keys=True) for step in step_list[:cut]]


def audit(document: dict, plan: dict) -> list[str]:
    problems: list[str] = []
    jobs = document.get("jobs") or {}
    for missing in unplanned_shards(jobs, plan):
        problems.append(f"{missing}: runs as a shard but the plan does not name it, "
                        f"so nothing checks what it runs or how it is balanced")
    for group, lanes in plan_groups(plan).items():
        mode = group_mode(group, lanes, plan)
        planned: dict[str, str] = {}
        for lane in lanes:
            for step in plan["lanes"][lane].get("steps", []):
                name = step["name"]
                if name in planned:
                    problems.append(f"{group}: {name!r} is planned in both "
                                    f"{planned[name]} and {lane}")
                planned[name] = lane
        if not planned:
            problems.append(f"{group}: no gates planned")
            continue
        for lane in lanes:
            if lane not in jobs:
                problems.append(f"{group}: plan names lane {lane!r}, the workflow does not")
                continue
            steps = jobs[lane].get("steps") or []
            known = set(planned)
            # Everything before the first planned gate is preamble; only the
            # steps after it can be gates (in `shell` mode the shard step is
            # itself the only thing after the preamble).  The plan records the
            # shard step by name, so a renamed or vanished step is a failure
            # instead of a silent fall-through to the ERR-trap search.
            recorded = plan["lanes"][lane].get("step")
            if recorded:
                start = next((i for i, step in enumerate(steps) if step_name(step) == recorded),
                             None)
                if start is None:
                    problems.append(f"{lane}: the plan's shard step {recorded!r} does not run here")
                    continue
            else:
                start = gate_start(steps, known, mode)
            present, unknown = [], []
            for step in steps[start:]:
                for gate in shard_gates(step, known, mode):
                    present.append(gate)
                    if gate not in known:
                        unknown.append(gate)
            for gate in unknown:
                problems.append(f"{lane}: {gate!r} runs here but the plan does not "
                                f"know it: a hand-added gate bypasses the balance "
                                f"and the review record")
            expected = [step["name"] for step in plan["lanes"][lane]["steps"]]
            for gate in expected:
                if gate not in present:
                    problems.append(f"{lane}: planned gate {gate!r} does not run here")
            for gate in set(present) - set(expected):
                if gate in planned and planned[gate] != lane:
                    problems.append(f"{lane}: gate {gate!r} is planned in {planned[gate]}")
        # Preamble identity: setup steps are the lane's identity, not work
        # that can be divided, so every shard must carry the same bytes.
        present_lanes = [lane for lane in lanes if lane in jobs]
        preambles = {lane: preamble(jobs[lane].get("steps") or [], known, mode)
                     for lane in present_lanes}
        for lane, steps_ in preambles.items():
            if steps_ != preambles[present_lanes[0]]:
                problems.append(f"{lane}: preamble differs from {present_lanes[0]}: a shard "
                                f"lost or gained setup")
        costs = [plan["lanes"][lane].get("predicted_gate_seconds", 0.0) for lane in lanes]
        mean = sum(costs) / len(costs)
        if mean and max(costs) - min(costs) > BALANCE_TOLERANCE * mean:
            problems.append(f"{group}: shards are imbalanced: "
                            + ", ".join(f"{lane} {cost:.0f} s" for lane, cost in zip(lanes, costs))
                            + f" (mean {mean:.0f} s, tolerance {BALANCE_TOLERANCE:.0%})")
    return problems


def self_test() -> None:
    def step(name: str, run: str = "true") -> dict:
        return {"name": name, "run": run}

    setup = [step("Checkout"), step("Install Rust")]
    doc = {"jobs": {
        "lane-a": {"steps": setup + [step("gate 1"), step("gate 2")]},
        "lane-b": {"steps": setup + [step("gate 3")]},
    }}
    plan = {
        "mode": "steps",
        "lanes": {
            "lane-a": {"group": "lane", "predicted_gate_seconds": 10.0, "steps":
                       [{"name": "gate 1", "measured_s": 5.0},
                        {"name": "gate 2", "measured_s": 5.0}]},
            "lane-b": {"group": "lane", "predicted_gate_seconds": 10.5, "steps":
                       [{"name": "gate 3", "measured_s": 10.5}]},
        },
    }
    assert audit(doc, plan) == [], audit(doc, plan)

    # A gate moved to the other shard: the plan and the workflow disagree.
    moved = json.loads(json.dumps(doc))
    moved["jobs"]["lane-b"]["steps"].insert(2, step("gate 1"))
    moved["jobs"]["lane-a"]["steps"] = setup + [step("gate 2")]
    assert any("does not run here" in p for p in audit(moved, plan)), audit(moved, plan)

    # A gate deleted from the workflow entirely.
    gone = json.loads(json.dumps(doc))
    gone["jobs"]["lane-a"]["steps"] = setup + [step("gate 1")]
    assert any("gate 2" in p and "does not run" in p for p in audit(gone, plan)), \
        audit(gone, plan)

    # An unplanned step added by hand.
    extra = json.loads(json.dumps(doc))
    extra["jobs"]["lane-b"]["steps"] = setup + [step("gate 3"), step("gate smuggled")]
    assert any("does not" in p and "know it" in p for p in audit(extra, plan)), \
        audit(extra, plan)

    # A shard that lost a preamble step (a cache, an oracle probe).
    drift = json.loads(json.dumps(doc))
    drift["jobs"]["lane-b"]["steps"] = [step("Checkout")] + [step("gate 3")]
    assert any("preamble differs" in p for p in audit(drift, plan)), audit(drift, plan)

    # Balance wrecked: the split no longer buys parallelism.
    skewed = json.loads(json.dumps(plan))
    skewed["lanes"]["lane-b"]["predicted_gate_seconds"] = 40.0
    assert any("imbalanced" in p for p in audit(doc, skewed)), audit(doc, skewed)

    # Shell mode: gates are logical commands inside one run body.
    body = ("        set -Eeuo pipefail\n"
            "        f() {\n"
            "        trap 'rc=$?; exit \"$rc\"' ERR\n"
            "        # a gate\n"
            "        python3 scripts/one.py\n"
            "        bash tests/regression/two.sh --flag\n"
            "        }\n"
            "        if ! f 2>&1 | tee log; then exit 1; fi\n")
    shell_doc = {"jobs": {"lane-a": {"steps": [step("Checkout"),
                                                 {"name": "shard a", "run": body}]}}}
    shell_plan = {"mode": "shell", "lanes": {"lane-a": {
        "group": "lane", "predicted_gate_seconds": 2.0,
        "steps": [{"name": "python3 scripts/one.py", "measured_s": 1.0},
                  {"name": "bash tests/regression/two.sh --flag", "measured_s": 1.0}]}}}
    assert shard_gates(shell_doc["jobs"]["lane-a"]["steps"][1], set(), "shell") == [
        "python3 scripts/one.py", "bash tests/regression/two.sh --flag"], \
        shard_gates(shell_doc["jobs"]["lane-a"]["steps"][1], set(), "shell")
    assert audit(shell_doc, shell_plan) == [], audit(shell_doc, shell_plan)

    # Two shapes in ONE plan: a step lane beside a shell lane.  A file-level
    # mode cannot read this file -- the step grammar sees the shell lane's one
    # step and reports every planned command as missing -- so the mode is read
    # per lane, and this fixture is what pins it.
    mixed_plan = {"lanes": json.loads(json.dumps(plan["lanes"])), "provenance": {}}
    mixed_doc = {"jobs": json.loads(json.dumps(doc["jobs"]))}
    for lane, spec in shell_plan["lanes"].items():
        shell_lane = lane.replace("lane-", "shell-")
        mixed_plan["lanes"][shell_lane] = dict(spec, group="shell")
        mixed_doc["jobs"][shell_lane] = shell_doc["jobs"][lane]
    for lane, spec in mixed_plan["lanes"].items():
        spec["mode"] = "shell" if lane.startswith("shell-") else "steps"
    assert audit(mixed_doc, mixed_plan) == [], audit(mixed_doc, mixed_plan)

    # The rule that caught the shipped bug: two sibling jobs with their own
    # steps that no plan names.  They ran 139 gates with no recorded
    # partition, because the record was keyed by step name and the next
    # lane's split pruned them as stale.
    unplanned_doc = {"jobs": dict(doc["jobs"], **shell_doc["jobs"],
                                  **{"orphan-a": {"steps": [step("gate 9")]},
                                     "orphan-b": {"steps": [step("gate 10")]}})}
    problems = audit(unplanned_doc, plan)
    assert any("orphan-a" in problem and "does not name it" in problem
               for problem in problems), problems
    assert any("orphan-b" in problem and "does not name it" in problem
               for problem in problems), problems
    print("check_ci_lane_plan: self-test PASS (8 rules, 11 fixtures)")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="verify the sharded lane partition")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--workflow", type=Path, default=WORKFLOW)
    parser.add_argument("--plan", type=Path, default=PLAN)
    args = parser.parse_args(argv)
    if args.self_test:
        self_test()
        return 0
    try:
        import yaml
    except ImportError:
        print("check_ci_lane_plan: PyYAML is required (python3-yaml)", file=sys.stderr)
        return 2
    if not args.plan.is_file():
        print(f"missing {args.plan}: run scripts/ci_lane_split.py", file=sys.stderr)
        return 1
    document = yaml.safe_load(args.workflow.read_text())
    plan = json.loads(args.plan.read_text())
    problems = audit(document, plan)
    if problems:
        print("sharded lane plan violated:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        return 1
    groups = plan_groups(plan)
    total = sum(spec.get("predicted_gate_seconds", 0.0) for spec in plan["lanes"].values())
    print("sharded lane plan: PASS ("
          + ", ".join(f"{g}={len(l)} shards" for g, l in groups.items())
          + f", {total:.0f} s of gate cost)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
