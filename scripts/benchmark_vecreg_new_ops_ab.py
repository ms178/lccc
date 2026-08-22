#!/usr/bin/env python3
"""Paired VM screen for RA-21 register-resident SSE intrinsic chains.

The treatment uses the default compiler pipeline; the same compiler builds the
control with CCC_NO_VECREG=1.  This records CPU-pinned wall time and a paired
bootstrap interval.  It is deterministic non-PMU screening, not a bare-metal
cycle or hardware-counter claim.
"""

from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import tempfile
import time

EXPECTED = "3460498931314901991"


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="target/fastbuild/lccc")
    parser.add_argument(
        "--source", default="tests/benchmark/programs/vecreg_new_ops.c"
    )
    parser.add_argument("--output", default="vecreg-new-ops-timing.json")
    parser.add_argument("--pairs", type=int, default=24)
    parser.add_argument("--iterations", type=int, default=50_000_000)
    parser.add_argument("--bootstrap", type=int, default=20_000)
    parser.add_argument("--seed", type=lambda value: int(value, 0), default=0x21A)
    parser.add_argument("--cpu", type=int, default=0)
    args = parser.parse_args()
    if args.pairs < 2 or args.iterations < 1 or args.bootstrap < 100:
        parser.error("pairs >= 2, iterations >= 1, and bootstrap >= 100 required")
    return args


def compile_variant(compiler: Path, source: Path, output: Path, *, stack: bool) -> None:
    env = os.environ.copy()
    if stack:
        env["CCC_NO_VECREG"] = "1"
    subprocess.run(
        [str(compiler), "-O3", "-msse4.1", str(source), "-o", str(output)],
        env=env,
        check=True,
    )


def timed_run(binary: Path, cpu: int, iterations: int) -> int:
    if shutil.which("taskset") is None:
        raise SystemExit("taskset is required for CPU-pinned paired measurements")
    start = time.perf_counter_ns()
    result = subprocess.run(
        ["taskset", "-c", str(cpu), str(binary), str(iterations)],
        capture_output=True,
        text=True,
    )
    elapsed = time.perf_counter_ns() - start
    if result.returncode != 0 or result.stdout.strip() != EXPECTED:
        raise SystemExit(
            f"bad result from {binary}: exit={result.returncode}, "
            f"stdout={result.stdout!r}, stderr={result.stderr!r}"
        )
    return elapsed


def distribution(values: list[int]) -> dict[str, float | int]:
    return {
        "arithmetic_mean": statistics.mean(values),
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
    }


def main() -> None:
    args = arguments()
    compiler = Path(args.compiler).resolve()
    source = Path(args.source).resolve()
    if not compiler.is_file() or not source.is_file():
        raise SystemExit(f"missing compiler or source: {compiler}, {source}")

    rng = random.Random(args.seed)
    with tempfile.TemporaryDirectory(prefix="lccc-vecreg-new-ops-") as tmp:
        tmpdir = Path(tmp)
        binaries = {
            "register": tmpdir / "register",
            "stack": tmpdir / "stack",
        }
        compile_variant(compiler, source, binaries["register"], stack=False)
        compile_variant(compiler, source, binaries["stack"], stack=True)
        for binary in binaries.values():
            timed_run(binary, args.cpu, 1000)

        raw: list[dict[str, object]] = []
        for pair in range(args.pairs):
            order = list(binaries)
            rng.shuffle(order)
            times: dict[str, int] = {}
            for name in order:
                times[name] = timed_run(
                    binaries[name], args.cpu, args.iterations
                )
            raw.append({"pair": pair, "order": order, "times_ns": times})

    log_ratios = [
        math.log(row["times_ns"]["register"] / row["times_ns"]["stack"])
        for row in raw
    ]
    bootstrap = sorted(
        math.exp(
            sum(log_ratios[rng.randrange(args.pairs)] for _ in range(args.pairs))
            / args.pairs
        )
        for _ in range(args.bootstrap)
    )
    register = [row["times_ns"]["register"] for row in raw]
    stack = [row["times_ns"]["stack"] for row in raw]
    low = bootstrap[int(0.025 * args.bootstrap)]
    high = bootstrap[min(args.bootstrap - 1, int(0.975 * args.bootstrap))]
    ratio = math.exp(statistics.mean(log_ratios))
    summary = {
        "evidence_scope": "CPU-pinned VM wall-time screen; no PMU or bare-metal claim",
        "kernel": f"multi-use 128-bit saturation/average chain, {args.iterations} iterations",
        "compiler": str(compiler),
        "control": "same compiler with CCC_NO_VECREG=1",
        "cpu": args.cpu,
        "pairs": args.pairs,
        "seed": args.seed,
        "register_ns": distribution(register),
        "stack_ns": distribution(stack),
        "paired_register_over_stack_geomean": ratio,
        "paired_bootstrap_95pct_ci": [low, high],
        "register_faster_pairs": sum(a < b for a, b in zip(register, stack)),
        "conclusion": (
            "register path faster in this screen"
            if high < 1.0
            else "stack path faster in this screen"
            if low > 1.0
            else "statistically unresolved in this screen"
        ),
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({"summary": summary, "raw": raw}, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
