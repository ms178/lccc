#!/usr/bin/env python3
"""Randomized paired VM screen for scalar FP memory-source folding.

Builds the same stencil kernel twice with one compiler binary.  The control
sets CCC_PEEPHOLE_SKIP=fp_reg_mem_fold; the treatment uses the default pass
pipeline.  Results are screening evidence, not PMU-backed bare-metal proof.
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


def arguments() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="target/fastbuild/lccc")
    parser.add_argument(
        "--source", default="tests/benchmark/programs/fp_memfold_stencil5.c"
    )
    parser.add_argument("--output", default="fp-memfold-stencil5-timing.json")
    parser.add_argument("--pairs", type=int, default=24)
    parser.add_argument("--repetitions", type=int, default=1000)
    parser.add_argument("--bootstrap", type=int, default=20000)
    parser.add_argument("--seed", type=lambda value: int(value, 0), default=0x14700)
    parser.add_argument("--cpu", type=int, default=0)
    args = parser.parse_args()
    if args.pairs < 2 or args.repetitions < 1 or args.bootstrap < 100:
        parser.error("pairs >= 2, repetitions >= 1, and bootstrap >= 100 required")
    return args


def compile_variant(
    compiler: Path, source: Path, output: Path, *, disable_fold: bool
) -> None:
    env = os.environ.copy()
    if disable_fold:
        old = env.get("CCC_PEEPHOLE_SKIP", "")
        env["CCC_PEEPHOLE_SKIP"] = ",".join(
            item for item in (old, "fp_reg_mem_fold") if item
        )
    subprocess.run(
        [
            str(compiler),
            "-O3",
            "-march=x86-64-v3",
            "-ffast-math",
            str(source),
            "-o",
            str(output),
        ],
        env=env,
        check=True,
    )


def command(binary: Path, cpu: int, repetitions: int) -> list[str]:
    if shutil.which("taskset") is None:
        raise SystemExit("taskset is required for CPU-pinned paired measurements")
    return ["taskset", "-c", str(cpu), str(binary), str(repetitions)]


def run_checked(binary: Path, cpu: int, repetitions: int) -> int:
    start = time.perf_counter_ns()
    result = subprocess.run(
        command(binary, cpu, repetitions), capture_output=True, text=True
    )
    elapsed = time.perf_counter_ns() - start
    if result.returncode != 0 or result.stdout.strip() != "614362":
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
    with tempfile.TemporaryDirectory(prefix="lccc-fp-memfold-") as tmp:
        tmpdir = Path(tmp)
        binaries = {
            "fp-mem-fold": tmpdir / "folded",
            "register-loads": tmpdir / "unfolded",
        }
        compile_variant(compiler, source, binaries["fp-mem-fold"], disable_fold=False)
        compile_variant(compiler, source, binaries["register-loads"], disable_fold=True)
        for binary in binaries.values():
            run_checked(binary, args.cpu, 10)

        raw = []
        for pair in range(args.pairs):
            order = list(binaries)
            rng.shuffle(order)
            record: dict[str, object] = {"pair": pair, "order": order, "times_ns": {}}
            times = record["times_ns"]
            assert isinstance(times, dict)
            for name in order:
                times[name] = run_checked(binaries[name], args.cpu, args.repetitions)
            raw.append(record)

    log_ratios = [
        math.log(row["times_ns"]["fp-mem-fold"] / row["times_ns"]["register-loads"])
        for row in raw
    ]
    bootstrap = sorted(
        math.exp(
            sum(log_ratios[rng.randrange(args.pairs)] for _ in range(args.pairs))
            / args.pairs
        )
        for _ in range(args.bootstrap)
    )
    low = bootstrap[int(0.025 * args.bootstrap)]
    high = bootstrap[min(args.bootstrap - 1, int(0.975 * args.bootstrap))]
    folded = [row["times_ns"]["fp-mem-fold"] for row in raw]
    unfolded = [row["times_ns"]["register-loads"] for row in raw]
    ratio = math.exp(statistics.mean(log_ratios))
    summary = {
        "evidence_scope": "CPU-pinned wall-time screening; no PMU or bare-metal claim",
        "kernel": f"65536-element five-point F32 stencil, {args.repetitions} calls/sample",
        "compiler": str(compiler),
        "control": "same compiler with CCC_PEEPHOLE_SKIP=fp_reg_mem_fold",
        "cpu": args.cpu,
        "pairs": args.pairs,
        "seed": args.seed,
        "fp_mem_fold_ns": distribution(folded),
        "register_loads_ns": distribution(unfolded),
        "paired_fp_mem_fold_over_register_loads_geomean": ratio,
        "paired_bootstrap_95pct_ci": [low, high],
        "fp_mem_fold_faster_pairs": sum(a < b for a, b in zip(folded, unfolded)),
        "conclusion": (
            "folded faster in this screen"
            if high < 1.0
            else "unfolded faster in this screen"
            if low > 1.0
            else "statistically unresolved in this screen"
        ),
    }
    result = {"summary": summary, "raw": raw}
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
