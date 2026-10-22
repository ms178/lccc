#!/usr/bin/env python3
"""The two halves of the fast local contract suite must be a partition.

`Verify remaining fast local contracts` was one 703 s step: 140 independent
gate invocations run back to back, so the backend lane's wall clock was the sum
of every gate.  It is now two steps in two jobs, split by measured cost (the
budget table is in the comment above each step).  That split has exactly one
dangerous failure mode, and it is silent in both directions:

* a gate listed in BOTH halves runs twice, costs twice, and still shows green;
* a gate listed in NEITHER half disappears from hosted CI, and only
  `scripts/check_ci_gate_parity.py` would notice -- provided the gate is also
  registered in `scripts/ci_local.sh`, which is exactly the property a hand
  edit during a merge is most likely to break.

This gate pins the partition structurally, from the workflow file itself:

1. the two shard steps' gate sets are DISJOINT (nothing pays twice),
2. neither shard is empty (a typo in a step name must not turn into "0 gates
   ran, green"),
3. no shard gate also appears in another run step of the same workflow (that
   is a duplicate run across jobs, and it is invisible in the job matrix),
4. every shard gate is a registered fast gate in `scripts/ci_local.sh` (the
   local mirror is the register of record; parity then guarantees the union
   covers the register, so the two checks together close both directions).

5. the scaffolding that runs a shard body actually FAILS when a gate inside
   it fails.  The bodies run as a shell function under `set -Eeuo pipefail`
   with an ERR trap -- but bash switches both off inside a function whose
   exit status is being tested, so the natural-looking
   `if ! fast_contracts | tee log` form silently absorbs every failure
   except the body's last command.  That shipped, and while it did, a real
   census increase on main reported green here.  A partition check alone
   cannot see that (the gates are all *listed*; they just cannot fail), so
   this check additionally EXECUTES each shard's scaffolding with a
   synthetic body -- a failing probe followed by a marker -- and requires a
   non-zero exit with the marker never printed, plus a passing probe that
   does reach the marker.  The scaffolding is taken from the workflow text
   itself, so any future edit that re-tests the call is caught here, on the
   first run, by construction.

`--self-test` exercises each of those four rules against synthetic fixtures,
including the near-miss shapes (a gate moved, a gate copied, a gate renamed),
because a checker that cannot fail is decoration.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
LOCAL = ROOT / "scripts" / "ci_local.sh"

SHARD_PREFIX = "Verify remaining fast local contracts (shard"
TRAP = re.compile(r"^\s*trap\b.*\bERR\b", re.MULTILINE)
# A gate is a checked-in program the mirrors execute directly.  The spelling is
# the repository's own: every regression gate lives under tests/regression or
# is a scripts/*.py tool.
GATE = re.compile(r"(?:tests/regression/[A-Za-z0-9_./-]+\.(?:sh|py)"
                  r"|scripts/[A-Za-z0-9_./-]+\.py"
                  r"|tests/linker/[A-Za-z0-9_./-]+\.py)")
CONTINUES = re.compile(r"\\\s*$")
REGISTRATION = re.compile(r'^\s*gate\s+"([^"]+)"\s+(fast|slow)\b(.*)$')


def logical_commands(body: str) -> list[str]:
    """Join backslash continuations; drop comments and blank lines."""
    commands: list[str] = []
    buffer: list[str] = []
    for line in body.splitlines():
        if not buffer:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
        buffer.append(line)
        if not CONTINUES.search(line):
            commands.append("\n".join(buffer))
            buffer = []
    if buffer:
        commands.append("\n".join(buffer))
    return commands


def gate_region(body: str) -> list[str]:
    """The gate commands of a shard step: after the ERR trap, before the brace.

    The scaffolding around them (`set -Eeuo pipefail`, the `fast_contracts() {`
    wrapper, the tee/annotation lines that name `scripts/ci_annotate_log.py`)
    runs in EVERY shard by design, so counting it as a gate would make the
    partition impossible to satisfy and would hide the real duplicates.
    """
    commands = logical_commands(body)
    try:
        start = next(i for i, command in enumerate(commands) if TRAP.match(command))
    except StopIteration:
        return commands
    end = next((i for i in range(start + 1, len(commands)) if commands[i].strip() == "}"),
               len(commands))
    return commands[start + 1:end]


def gates_of(body: str) -> set[str]:
    found: set[str] = set()
    for command in gate_region(body):
        found.update(GATE.findall(command))
    return found


def invocations_of(body: str) -> set[str]:
    """Normalised gate invocations, so `selftest` and `verify-index` differ."""
    return {" ".join(command.split()) for command in gate_region(body)
            if GATE.search(command)}


def shard_steps(document: dict) -> list[tuple[str, set[str]]]:
    """(step name, gate set) for every shard step, in file order."""
    shards: list[tuple[str, set[str]]] = []
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            name = step.get("name") or ""
            if name.startswith(SHARD_PREFIX) and isinstance(step.get("run"), str):
                shards.append((name, gates_of(step["run"])))
    return shards


def document_step(name: str, document: dict) -> str:
    """The raw run body of the step called `name`."""
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            if (step.get("name") or "") == name and isinstance(step.get("run"), str):
                return step["run"]
    raise KeyError(name)


def other_steps(document: dict) -> dict[str, list[str]]:
    """normalised invocation -> step names that run it outside the shard steps."""
    owners: dict[str, list[str]] = {}
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            name = step.get("name") or step.get("uses") or "<unnamed>"
            if name.startswith(SHARD_PREFIX) or not isinstance(step.get("run"), str):
                continue
            for invocation in invocations_of(step["run"]):
                owners.setdefault(invocation, []).append(name)
    return owners


def registered_gates(text: str, *, speed: str = "fast") -> set[str]:
    """Gate programs `scripts/ci_local.sh` registers at the given speed."""
    lines = text.splitlines()
    registered: set[str] = set()
    index = 0
    while index < len(lines):
        match = REGISTRATION.match(lines[index])
        index += 1
        if not match or match.group(2) != speed:
            continue
        block = [match.group(3)]
        while CONTINUES.search(block[-1]) and index < len(lines):
            block.append(lines[index])
            index += 1
        registered.update(GATE.findall("\n".join(block)))
    return registered


def audit(document: dict, local_text: str) -> list[str]:
    """Every violation of the partition contract, as human-readable text."""
    problems: list[str] = []
    shards = shard_steps(document)
    if len(shards) < 2:
        return [f"expected at least 2 steps named '<{SHARD_PREFIX}...>', found {len(shards)}"]
    seen: dict[str, str] = {}
    programs: set[str] = set()
    for name, gates in shards:
        if not gates:
            problems.append(f"{name} runs no gate at all")
        for invocation in sorted(invocations_of(document_step(name, document))):
            if invocation in seen:
                problems.append(f"{invocation!r} is in both {seen[invocation]!r} "
                                f"and {name!r}")
            seen[invocation] = name
        programs |= gates
    owners = other_steps(document)
    for invocation in sorted(seen):
        if invocation in owners:
            problems.append(f"{invocation!r} also runs in {owners[invocation][0]!r}: "
                            f"the shard split must own it exclusively")
    registered = registered_gates(local_text)
    for gate in sorted(programs - registered):
        problems.append(f"{gate} runs in a shard but {LOCAL.name} does not register "
                        f"it as a fast gate: the mirrors have drifted")
    return problems


def scaffold_problems(document: dict) -> list[str]:
    """Execute each shard step's scaffolding with a synthetic body (rule 5).

    The shard body between the ERR trap and the closing brace is replaced by
    a probe: a failing command followed by a marker echo must abort the body
    (non-zero exit, marker never printed); a succeeding probe must reach its
    marker with exit 0.  Everything else -- the `set` line, the function
    wrapper, the trap, the tee/annotate tail -- is the workflow's own text,
    so what is being tested is exactly what CI will run.
    """
    bash = shutil.which("bash")
    if bash is None:
        return ["bash is not available, so the shard scaffolding cannot be executed"]
    problems: list[str] = []
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            name = step.get("name") or ""
            if not name.startswith(SHARD_PREFIX) or not isinstance(step.get("run"), str):
                continue
            commands = logical_commands(step["run"])
            trap_at = next((i for i, c in enumerate(commands) if TRAP.match(c)), None)
            brace_at = next((i for i in range((trap_at or 0) + 1, len(commands))
                             if commands[i].strip() == "}"), None)
            if trap_at is None or brace_at is None:
                problems.append(f"{name}: shard scaffolding (ERR trap / closing brace) "
                                f"not recognised, so its failure semantics are unpinned")
                continue
            head = commands[:trap_at + 1]
            tail = commands[brace_at:]
            for probe, want_fail in (("false", True), ("true", False)):
                marker = "SCAFFOLD_PROBE_MARKER"
                script = "\n".join(head + [probe, f"echo {marker}"] + tail) + "\n"
                with tempfile.TemporaryDirectory() as tmp:
                    env = dict(os.environ, RUNNER_TEMP=tmp)
                    try:
                        done = subprocess.run(
                            [bash, "-c", script], cwd=ROOT, env=env,
                            capture_output=True, text=True, timeout=60)
                    except subprocess.TimeoutExpired:
                        problems.append(f"{name}: synthetic `{probe}` body timed out")
                        continue
                if want_fail:
                    if done.returncode == 0 or marker in done.stdout:
                        problems.append(
                            f"{name}: a failing gate inside the shard body does not "
                            f"fail the step (exit {done.returncode}, marker "
                            f"{'printed' if marker in done.stdout else 'not printed'}): "
                            f"the body function must be called as a plain command, "
                            f"never in a tested context (`if !`, `&&`, `||`), where "
                            f"bash disables -e and the ERR trap inside it")
                elif done.returncode != 0 or marker not in done.stdout:
                    problems.append(
                        f"{name}: a passing shard body does not run to completion "
                        f"(exit {done.returncode}) under the step scaffolding")
    return problems


def self_test() -> None:
    def step(name: str, *commands: str) -> dict:
        return {"name": name, "run": "\n".join(commands)}

    local = ('gate "a" fast python3 scripts/check_a.py\n'
             'gate "b" fast bash tests/regression/check_b.sh\n'
             'gate "c" slow bash tests/regression/check_c.sh\n'
             'gate "d" fast \\\n    env CCC=x python3 tests/regression/check_d.py\n')

    def doc(*steps: dict) -> dict:
        return {"jobs": {"one": {"steps": list(steps)}}}

    clean = doc(step(SHARD_PREFIX + " a of 2)", "python3 scripts/check_a.py"),
                step(SHARD_PREFIX + " b of 2)", "bash tests/regression/check_b.sh",
                     "env CCC=x python3 tests/regression/check_d.py"))
    assert audit(clean, local) == [], audit(clean, local)

    duplicated = doc(step(SHARD_PREFIX + " a of 2)", "python3 scripts/check_a.py"),
                     step(SHARD_PREFIX + " b of 2)", "python3 scripts/check_a.py"))
    assert any("in both" in problem for problem in audit(duplicated, local)), \
        audit(duplicated, local)

    moved = doc(step(SHARD_PREFIX + " a of 2)", "python3 scripts/check_a.py"),
                step(SHARD_PREFIX + " b of 2)", "bash tests/regression/check_b.sh"),
                step("Backend pre-gates", "bash tests/regression/check_b.sh"))
    assert any("also runs in" in problem for problem in audit(moved, local)), audit(moved, local)

    unregistered = doc(step(SHARD_PREFIX + " a of 2)", "python3 scripts/check_a.py"),
                       step(SHARD_PREFIX + " b of 2)",
                            "bash tests/regression/check_ghost.sh"))
    assert any("does not register" in problem for problem in audit(unregistered, local)), \
        audit(unregistered, local)

    # A slow gate is not mirrored at --fast speed, so its registration does not
    # count: the shard job would be running something the fast register denies.
    slow_only = doc(step(SHARD_PREFIX + " a of 2)", "python3 scripts/check_a.py"),
                    step(SHARD_PREFIX + " b of 2)",
                         "bash tests/regression/check_c.sh"))
    assert any("check_c.sh" in problem for problem in audit(slow_only, local)), \
        audit(slow_only, local)

    empty = doc(step(SHARD_PREFIX + " a of 2)", "# nothing"),
                step(SHARD_PREFIX + " b of 2)", "python3 scripts/check_a.py"))
    assert any("runs no gate" in problem for problem in audit(empty, local)), audit(empty, local)

    single = doc(step(SHARD_PREFIX + " a of 2)", "python3 scripts/check_a.py"))
    assert any("at least 2" in problem for problem in audit(single, local)), audit(single, local)

    # Continuations and comments must not confuse the gate extractor.
    assert gates_of("# python3 scripts/not_a_gate.py\nenv A=1 \\\n  bash tests/regression/x.sh\n") == {
        "tests/regression/x.sh"}, gates_of("# c\nenv A=1 \\\n  bash tests/regression/x.sh\n")
    print("check_ci_shard_partition: self-test PASS (4 rules, 7 fixtures)")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="verify the fast-contract shard partition")
    parser.add_argument("--self-test", action="store_true", help="check the checker and exit")
    parser.add_argument("--workflow", type=Path, default=WORKFLOW)
    parser.add_argument("--local", type=Path, default=LOCAL)
    args = parser.parse_args(argv)
    if args.self_test:
        self_test()
        return 0
    try:
        import yaml
    except ImportError:
        print("check_ci_shard_partition: PyYAML is required (python3-yaml)", file=sys.stderr)
        return 2
    document = yaml.safe_load(args.workflow.read_text())
    problems = audit(document, args.local.read_text())
    problems += scaffold_problems(document)
    shards = shard_steps(document)
    if problems:
        for problem in problems:
            print(f"FAIL  {problem}", file=sys.stderr)
        return 1
    sizes = ", ".join(f"{name.rsplit('(', 1)[1].rstrip(')')}={len(gates)}"
                      for name, gates in shards)
    print(f"fast contract shard partition: PASS ({sizes} gates, disjoint, "
          f"each owned by one step, all registered in {args.local.name}; "
          f"scaffolding executed with failing and passing probes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
