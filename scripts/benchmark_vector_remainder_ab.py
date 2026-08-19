#!/usr/bin/env python3
"""Paired VM screen for division-free vector remainder transitions.

The treatment and control are built from separate compiler paths so a clean
upstream compiler can be compared with the current compiler. Raw CPU-pinned
wall-time samples and a paired bootstrap interval are retained. This is VM
screening, not PMU or bare-metal evidence.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
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
    parser.add_argument("--baseline-compiler", required=True)
    parser.add_argument(
        "--source", default="tests/benchmark/programs/vector_remainder.c"
    )
    parser.add_argument("--output", default="vector-remainder-timing.json")
    parser.add_argument("--pairs", type=int, default=24)
    parser.add_argument("--repetitions", type=int, default=200000)
    parser.add_argument("--bootstrap", type=int, default=20000)
    parser.add_argument("--seed", type=lambda value: int(value, 0), default=0x14700)
    parser.add_argument("--cpu", type=int, default=0)
    args = parser.parse_args()
    if args.pairs < 2 or args.repetitions < 1 or args.bootstrap < 100:
        parser.error("pairs >= 2, repetitions >= 1, and bootstrap >= 100 required")
    return args


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compile_variant(compiler: Path, source: Path, output: Path) -> None:
    subprocess.run(
        [
            str(compiler),
            "-O3",
            "-march=x86-64-v3",
            "-ffast-math",
            "-ffp-contract=fast",
            str(source),
            "-o",
            str(output),
        ],
        check=True,
    )


def command(binary: Path, cpu: int, repetitions: int) -> list[str]:
    if shutil.which("taskset") is None:
        raise SystemExit("taskset is required for CPU-pinned paired measurements")
    return ["taskset", "-c", str(cpu), str(binary), str(repetitions)]


def run_checked(binary: Path, cpu: int, repetitions: int, expected: str) -> int:
    start = time.perf_counter_ns()
    result = subprocess.run(
        command(binary, cpu, repetitions), capture_output=True, text=True
    )
    elapsed = time.perf_counter_ns() - start
    if result.returncode != 0 or result.stdout.strip() != expected:
        raise SystemExit(
            f"bad result from {binary}: exit={result.returncode}, "
            f"stdout={result.stdout!r}, expected={expected!r}, "
            f"stderr={result.stderr!r}"
        )
    return elapsed


def output_for(binary: Path, cpu: int, repetitions: int) -> str:
    result = subprocess.run(
        command(binary, cpu, repetitions), capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def distribution(values: list[int]) -> dict[str, float | int]:
    return {
        "arithmetic_mean": statistics.mean(values),
        "median": statistics.median(values),
        "min": min(values),
        "max": max(values),
    }


def main() -> None:
    args = arguments()
    compilers = {
        "division-free": Path(args.compiler).resolve(),
        "signed-division": Path(args.baseline_compiler).resolve(),
    }
    source = Path(args.source).resolve()
    if not source.is_file() or any(not path.is_file() for path in compilers.values()):
        raise SystemExit(f"missing source or compiler: {source}, {compilers}")

    rng = random.Random(args.seed)
    with tempfile.TemporaryDirectory(prefix="lccc-vector-remainder-") as tmp:
        tmpdir = Path(tmp)
        binaries = {name: tmpdir / name for name in compilers}
        for name, compiler in compilers.items():
            compile_variant(compiler, source, binaries[name])

        expected = output_for(binaries["signed-division"], args.cpu, args.repetitions)
        if output_for(binaries["division-free"], args.cpu, args.repetitions) != expected:
            raise SystemExit("compiler variants produced different results")
        for binary in binaries.values():
            run_checked(binary, args.cpu, 10, output_for(binary, args.cpu, 10))

        raw = []
        for pair in range(args.pairs):
            order = list(binaries)
            rng.shuffle(order)
            record: dict[str, object] = {"pair": pair, "order": order, "times_ns": {}}
            times = record["times_ns"]
            assert isinstance(times, dict)
            for name in order:
                times[name] = run_checked(
                    binaries[name], args.cpu, args.repetitions, expected
                )
            raw.append(record)

    treatment = [row["times_ns"]["division-free"] for row in raw]
    control = [row["times_ns"]["signed-division"] for row in raw]
    log_ratios = [math.log(a / b) for a, b in zip(treatment, control)]
    bootstrap = sorted(
        math.exp(
            sum(log_ratios[rng.randrange(args.pairs)] for _ in range(args.pairs))
            / args.pairs
        )
        for _ in range(args.bootstrap)
    )
    low = bootstrap[int(0.025 * args.bootstrap)]
    high = bootstrap[min(args.bootstrap - 1, int(0.975 * args.bootstrap))]
    ratio = math.exp(statistics.mean(log_ratios))
    summary = {
        "evidence_scope": "CPU-pinned VM wall-time screening; no PMU or bare-metal claim",
        "kernel": (
            "F64 sum+dot over 18 dynamic bounds straddling AVX2 widths, "
            f"{args.repetitions} outer repetitions/sample"
        ),
        "source": str(source),
        "compiler": str(compilers["division-free"]),
        "compiler_sha256": digest(compilers["division-free"]),
        "baseline_compiler": str(compilers["signed-division"]),
        "baseline_compiler_sha256": digest(compilers["signed-division"]),
        "cpu": args.cpu,
        "pairs": args.pairs,
        "seed": args.seed,
        "result": expected,
        "division_free_ns": distribution(treatment),
        "signed_division_ns": distribution(control),
        "paired_division_free_over_signed_division_geomean": ratio,
        "paired_bootstrap_95pct_ci": [low, high],
        "division_free_faster_pairs": sum(a < b for a, b in zip(treatment, control)),
        "conclusion": (
            "division-free variant faster in this screen"
            if high < 1.0
            else "signed-division variant faster in this screen"
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
