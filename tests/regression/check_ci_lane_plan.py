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



# ── oracle provisioning ────────────────────────────────────────────────────
# A lane's toolchain is an INPUT CONTRACT, and an input contract that is only
# checked implicitly -- by the first gate that trips over a missing tool -- is
# unverifiable: the failure names the consumer, not the dependency.
#
# That is not hypothetical.  Run 39809624659 spent a six-minute contract job to
# report "LCCC, GCC, Clang and Valgrind are all required" from the
# hexadecimal-literal oracle, because no lane installed valgrind (and the
# contract lanes never installed the pinned Clang 23.1.3 oracle either); the
# same run's sibling shard failed check_llvm_word_counts.py because the lane
# could receive any GCC 16.x while the oracle family pins 16.2.  Both are
# one-line provisioning facts that this audit can read from the workflow in
# under a tenth of a second, with no compiler and no network.
#
# The rules, each derived from what the gates THEMSELVES say they need:
#
#   R1  a job whose gates need the pinned Clang oracle installs it
#       (scripts/ci_install_clang_oracle.sh), instead of trusting whatever
#       `clang` the runner image happens to ship;
#   R2  a job whose gates run Memcheck provisions the toolchain, which is the
#       only path that installs valgrind + libc6-dbg;
#   R3  every job that provisions a toolchain VERIFIES it first
#       (scripts/ci_verify_oracles.sh): the lane must fail at minute 1 naming
#       the tool, not at minute 12 naming a gate;
#   R4  a lane that installs the pinned oracle also asserts its version in that
#       verification (--clang-version), so "the oracle" cannot silently be a
#       different release;
#   R5  the provisioning script's EXTRAS still carries valgrind and libc6-dbg.
#
# R1/R2 are derived from the gate scripts rather than from a hand-kept list: a
# new gate that pins the Clang oracle or runs Memcheck pulls its lane into the
# rule automatically, which is the property that keeps this audit honest.

CLANG_ORACLE_INSTALL = "scripts/ci_install_clang_oracle.sh"
TOOLCHAIN_PROVISION = "scripts/ci_provision_toolchain.sh"
ORACLE_VERIFY = "scripts/ci_verify_oracles.sh"
VERIFY_STEP = "Verify the lane's oracle toolchain"
PROVISIONING_SCRIPT = ROOT / TOOLCHAIN_PROVISION
REQUIRED_EXTRAS = ("valgrind", "libc6-dbg")
GATE_CALL = re.compile(r"\b(?:bash|sh|python3?)\s+(\S+\.(?:sh|py))")

# Files that DESCRIBE a tool requirement instead of consuming it: the
# provisioning script names the oracles it installs, the verifier names what it
# probes, and this audit names both.  Following the closure into them would
# make every lane that provisions look like a lane that runs Memcheck -- the
# false positive that turns a fail-closed audit into noise.
INFRASTRUCTURE = frozenset({
    "scripts/ci_provision_toolchain.sh",
    "scripts/ci_toolchain_bundle.sh",
    "scripts/ci_verify_oracles.sh",
    "scripts/ci_install_clang_oracle.sh",
    "tests/regression/check_ci_lane_plan.py",
})

# What a gate's own text says it needs.  A gate that refuses to run without the
# pinned Clang oracle says so by sourcing the shared policy; a gate that runs
# Memcheck says so by invoking the oracle scripts or by asking for the binary.
CLANG_IDIOMS = ("require_clang_oracle",)
VALGRIND_IDIOMS = ("which('valgrind')", 'which("valgrind")', "command -v valgrind",
                   "check_hex_float_oracles", "check_signed_mask_oracles")
# The hexadecimal oracle compares against Clang as well as GCC.
CLANG_ALSO = ("check_hex_float_oracles", "check_signed_mask_oracles")

_closure_cache: dict[str, frozenset[str]] = {}


def _repo_relative(token: str) -> str:
    """A repo-relative path from a shell token like `"$root/scripts/gate.py"`.

    Gate scripts resolve their own directory and invoke siblings through it
    (`python3 "$root/scripts/check_hex_float_oracles.py"`), so the token in the
    text is neither plain nor repo-relative.  Anchoring on the first `scripts/`
    or `tests/` component recovers the path the audit can open, which is what
    makes the hexadecimal oracle's valgrind requirement visible from the FMA
    contract gate that calls it.
    """
    token = token.strip("\"'")
    for anchor in ("scripts/", "tests/", "tools/"):
        index = token.find(anchor)
        if index >= 0:
            return token[index:]
    return token


def _gate_tools(relative: str, seen: frozenset[str] = frozenset()) -> frozenset[str]:
    """Tools a gate needs, following the gate scripts it invokes (depth 4)."""
    relative = _repo_relative(relative)
    if relative in _closure_cache:
        return _closure_cache[relative]
    if relative in seen or len(seen) > 4 or relative in INFRASTRUCTURE:
        return frozenset()
    path = ROOT / relative
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return frozenset()
    tools = set()
    if any(idiom in text for idiom in CLANG_IDIOMS):
        tools.add("clang")
    if any(idiom in text for idiom in VALGRIND_IDIOMS):
        tools.add("valgrind")
    if any(idiom in text for idiom in CLANG_ALSO):
        tools.add("clang")
    if not tools:
        for match in GATE_CALL.finditer(text):
            tools |= set(_gate_tools(match.group(1), seen | {relative}))
    _closure_cache[relative] = frozenset(tools)
    return _closure_cache[relative]


# A gate that asserts the minor version does so on the compiler's own banner
# (`re.search(r"\b16\.2(?:\.0)?\b", gcc --version)`); the escaped form is
# what appears in the source, so it distinguishes a real pin from a version
# string in a comment.
GCC_PIN_IDIOM = r"16\.2"


def _pins_gcc_minor(relative: str) -> bool:
    try:
        return GCC_PIN_IDIOM in (ROOT / relative).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False


def oracle_provisioning(document: dict) -> list[str]:
    """R1-R4: every lane provisions the oracles its own gates demand."""
    problems: list[str] = []
    for name, job in sorted((document.get("jobs") or {}).items()):
        steps = job.get("steps") or []
        if not steps:
            continue  # reusable-workflow shard: its gates live elsewhere
        runs = [step.get("run") or "" for step in steps]
        joined = "\n".join(runs)
        tools: set[str] = set()
        for match in GATE_CALL.finditer(joined):
            tools |= set(_gate_tools(match.group(1)))
        installs_clang = CLANG_ORACLE_INSTALL in joined
        provisions = TOOLCHAIN_PROVISION in joined
        verifies = [run for step, run in zip(steps, runs)
                    if (step.get("name") or "") == VERIFY_STEP]
        if "clang" in tools and not installs_clang:
            problems.append(
                f"{name}: runs a gate that pins the Clang 23.1.3 oracle but no step "
                f"installs it ({CLANG_ORACLE_INSTALL}): the runner image's `clang` is "
                f"several releases older, so the gate would either fail or compare "
                f"against the wrong reference")
        if "valgrind" in tools and not provisions:
            problems.append(
                f"{name}: runs a Memcheck gate but no step provisions the toolchain "
                f"({TOOLCHAIN_PROVISION}), which is the only path that installs "
                f"valgrind + libc6-dbg")
        if provisions and not verifies:
            problems.append(
                f"{name}: provisions a toolchain but never verifies it "
                f"({VERIFY_STEP!r} running {ORACLE_VERIFY}): a missing or wrong tool "
                f"then surfaces minutes later, naming a gate instead of the tool")
        if installs_clang:
            for run in verifies:
                if "--clang-version" not in run:
                    problems.append(
                        f"{name}: installs the pinned Clang oracle but its "
                        f"{VERIFY_STEP!r} step does not assert --clang-version: "
                        f"'the oracle' must mean exactly 23.1.3")
        # R6: the lane runs a gate that pins the GCC minor, so the lane's own
        # probe must assert it too.  Without this, a lane can be provisioned
        # with 16.3 and pass its probe, only for the first pinning gate to fail
        # twelve minutes later with "GCC 16.2 required" and no version named.
        pins = [match.group(1) for match in GATE_CALL.finditer(joined)
                if _pins_gcc_minor(_repo_relative(match.group(1)))]
        if pins and provisions:
            for run in verifies:
                if "--gcc-version" not in run:
                    problems.append(
                        f"{name}: runs {_repo_relative(pins[0])!r}, which pins GCC 16.2, "
                        f"but its {VERIFY_STEP!r} step does not assert --gcc-version: "
                        f"the lane would pass with 16.3 and fail in a later gate")
    return problems


def provisioning_extras_problems(text: str) -> list[str]:
    """R5: the package set every bundle and every lane is built from."""
    match = re.search(r'^EXTRAS="([^"]*)"', text, re.M)
    if not match:
        return ["scripts/ci_provision_toolchain.sh: no EXTRAS= declaration: the "
                "audit cannot prove what a lane installs"]
    packages = match.group(1).split()
    return [f"scripts/ci_provision_toolchain.sh: EXTRAS lacks {package!r}: every "
            f"lane bundles this package set, so a Memcheck gate would run only "
            f"where the tool happened to be installed"
            for package in REQUIRED_EXTRAS if package not in packages]

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
    # The partition is only half of a lane's contract: the other half is that
    # the lane can RUN what it was given.
    problems.extend(oracle_provisioning(document))
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
    # ── oracle provisioning (R1-R5) ────────────────────────────────────────
    # A lane that runs a Memcheck gate and the pinned-Clang gate but installs
    # neither: exactly the shape run 39809624659 shipped.  All three rules must
    # fire, and each must name the job and the missing piece.
    bare = {"jobs": {"contracts": {"steps": [
        step("Checkout"),
        step("Run the contracts", "bash tests/regression/check_fma_gating.sh"),
    ]}}}
    problems = oracle_provisioning(bare)
    assert any("pins the Clang 23.1.3 oracle" in p and "contracts" in p for p in problems), problems
    assert any("Memcheck gate" in p and "contracts" in p for p in problems), problems

    # A lane that provisions and never verifies: the tool is there, but nothing
    # proves it is the pinned one until a gate fails twelve minutes later.
    unverified = {"jobs": {"contracts": {"steps": [
        step("Checkout"),
        step("Provision", "bash scripts/ci_provision_toolchain.sh --multilib"),
        step("Run the contracts", "bash tests/regression/check_fma_gating.sh"),
    ]}}}
    problems = oracle_provisioning(unverified)
    assert any(VERIFY_STEP in p and "never verifies" in p for p in problems), problems

    # Installing the oracle and verifying without asserting its version leaves
    # "the oracle" undefined: any release would pass.
    loose = {"jobs": {"contracts": {"steps": [
        step("Checkout"),
        step("Provision", "bash scripts/ci_provision_toolchain.sh --multilib"),
        step("Clang", "bash scripts/ci_install_clang_oracle.sh"),
        step(VERIFY_STEP, "bash scripts/ci_verify_oracles.sh --gcc-version 16.2"),
        step("Run the contracts", "bash tests/regression/check_fma_gating.sh"),
    ]}}}
    problems = oracle_provisioning(loose)
    assert any("--clang-version" in p for p in problems), problems

    # The fully provisioned lane is clean, so the rule can pass as well as fail.
    complete = {"jobs": {"contracts": {"steps": [
        step("Checkout"),
        step("Provision", "bash scripts/ci_provision_toolchain.sh --multilib"),
        step("Clang", "bash scripts/ci_install_clang_oracle.sh"),
        step(VERIFY_STEP, "bash scripts/ci_verify_oracles.sh --gcc-version 16.2 --clang-version 23.1.3"),
        step("Run the contracts", "bash tests/regression/check_fma_gating.sh"),
    ]}}}
    assert oracle_provisioning(complete) == [], oracle_provisioning(complete)

    # R6: the lane runs a gate that pins the GCC minor but its probe does not
    # assert it -- the shape that made run 39809624659's shard a fail inside
    # check_llvm_word_counts.py with no version in the message.
    gcc_loose = {"jobs": {"contracts": {"steps": [
        step("Checkout"),
        step("Provision", "bash scripts/ci_provision_toolchain.sh --multilib"),
        step("Clang", "bash scripts/ci_install_clang_oracle.sh"),
        step(VERIFY_STEP, "bash scripts/ci_verify_oracles.sh --clang-version 23.1.3"),
        step("Run the contracts", "bash tests/regression/check_llvm_word_counts.py"),
    ]}}}
    problems = oracle_provisioning(gcc_loose)
    assert any("--gcc-version" in p and "16.2" in p for p in problems), problems
    assert oracle_provisioning(complete) == [], oracle_provisioning(complete)

    # R5: the package set is what every bundle is built from.
    assert provisioning_extras_problems('EXTRAS="qemu-user mold lld"') != []
    assert provisioning_extras_problems('EXTRAS="qemu-user valgrind libc6-dbg"') == []
    assert provisioning_extras_problems("PACKAGES=qemu") != []

    print("check_ci_lane_plan: self-test PASS (15 rules, 19 fixtures)")


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
