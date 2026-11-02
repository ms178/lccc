#!/usr/bin/env python3
"""Measure what each CI gate costs, so a lane can be split on evidence.

The pipeline's wall clock is the longest lane, and a lane is a list of
independent gate invocations, so the only defensible way to divide one is to
know what each gate costs.  That number cannot be inferred from the workflow:
run 39752334266 showed `check_peephole_whitespace.sh` costing 337 s while a
stale table said 0.01 s (it had been timed in a workspace whose working
directory had been deleted under it), and the resulting split was 234 s / 527 s
instead of the predicted 191 s / 191 s.

This tool produces the table the splitter consumes, from the workflow itself:

    scripts/ci_gate_costs.py --lane backend-contracts-b --out costs.tsv

Each gate command is extracted from the lane's `run:` body exactly as the
shell would receive it (backslash continuations preserved, comments dropped),
then executed under `timeout` from the repository root with `CCC` pointing at
the built compiler.  A gate that fails still records its time: the cost of a
failing gate is the time it spends before failing, and pretending otherwise
would make the table a lie in precisely the runs that matter.

Output is `seconds<TAB>command`, one line per gate, sorted by cost descending,
so the heaviest gates are the first thing a reviewer sees.
"""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/ci.yml"
CONTINUES = "_CI_CONT"


def shard_bodies(path: Path, prefix: str) -> list[tuple[str, str]]:
    import yaml

    document = yaml.safe_load(path.read_text())
    found: list[tuple[str, str]] = []
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            name = step.get("name") or ""
            if name.startswith(prefix) and isinstance(step.get("run"), str):
                found.append((name, step["run"]))
    return found


def logical_commands(body: str) -> list[str]:
    """Join continuations, drop comments, keep the command the shell runs."""
    commands: list[str] = []
    buffer: list[str] = []
    for line in body.splitlines():
        if not buffer:
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
        buffer.append(line)
        if not line.rstrip().endswith("\\"):
            commands.append("\n".join(buffer))
            buffer = []
    if buffer:
        commands.append("\n".join(buffer))
    return commands


TRAP_MARK = "trap 'rc=$?"


def gate_region(body: str) -> list[str]:
    commands = logical_commands(body)
    start = next((i for i, c in enumerate(commands) if TRAP_MARK in c), None)
    if start is None:
        return commands
    end = next((i for i in range(start + 1, len(commands)) if commands[i].strip() == "}"),
               len(commands))
    return commands[start + 1:end]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lane", required=True,
                        help="step-name prefix, e.g. 'Verify remaining fast local contracts'")
    parser.add_argument("--workflow", type=Path, default=WORKFLOW)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=900.0, help="per-gate wall limit")
    parser.add_argument("--only", help="substring filter, for a focused re-measurement")
    parser.add_argument("--dry-run", action="store_true", help="list the gates and exit")
    args = parser.parse_args()

    bodies = shard_bodies(args.workflow, args.lane)
    if not bodies:
        print(f"no step named like {args.lane!r} in {args.workflow}", file=sys.stderr)
        return 2
    gates: list[str] = []
    for name, body in bodies:
        found = gate_region(body)
        print(f"{name}: {len(found)} gates", file=sys.stderr)
        gates.extend(found)
    if args.only:
        gates = [g for g in gates if args.only in g]
    if args.dry_run:
        for gate in gates:
            print(" ".join(gate.split()))
        return 0

    env = dict(os.environ)
    env.setdefault("CCC", str(ROOT / "target/fastbuild/lccc"))
    env.setdefault("CARGO_BUILD_JOBS", "2")
    env.pop("PYTHONPATH", None)

    rows: list[tuple[float, str, int]] = []
    for index, gate in enumerate(gates, 1):
        label = " ".join(gate.split())[:110]
        started = time.monotonic()
        try:
            result = subprocess.run(
                ["bash", "-c", gate], cwd=ROOT, env=env,
                stdin=subprocess.DEVNULL, capture_output=True, text=True,
                timeout=args.timeout,
            )
            status = result.returncode
        except subprocess.TimeoutExpired:
            status = 124
        seconds = time.monotonic() - started
        rows.append((seconds, gate, status))
        print(f"[{index}/{len(gates)}] {seconds:7.2f}s rc={status:3d} {label}", flush=True)

    rows.sort(key=lambda row: -row[0])
    args.out.write_text("# seconds\tcommand\trc (measured by scripts/ci_gate_costs.py)\n"
                        + "".join(f"{s:.2f}\t{' '.join(c.split())}\t{rc}\n"
                                  for s, c, rc in rows))
    total = sum(row[0] for row in rows)
    failed = [row for row in rows if row[2] != 0]
    print(f"\n{len(rows)} gates, {total:.1f}s measured, {len(failed)} non-zero exit")
    for seconds, gate, status in rows[:10]:
        print(f"  {seconds:8.2f}s rc={status:3d} {' '.join(gate.split())[:90]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
