#!/usr/bin/env python3
"""Measure scalar volatile-reduction ILP; never turn noisy timing into a CI gate.

All binaries must agree on stdout and successfully finish. Rotating compiler
order and a pinned CPU reduce, but do not eliminate, virtual-host noise. Pass
explicit GCC 16.2 / Clang 23.1.3 paths for reference comparisons.
"""
import argparse
import json
import os
from pathlib import Path
import resource
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def positive(text):
    number = int(text)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--gcc", type=Path)
    parser.add_argument("--clang", type=Path)
    parser.add_argument("--trips", type=positive, default=300000000)
    parser.add_argument("--samples", type=positive, default=9)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    cpu = None
    if hasattr(os, "sched_getaffinity"):
        cpu = min(os.sched_getaffinity(0))
        os.sched_setaffinity(0, {cpu})
    compilers = {"baseline": args.baseline, "candidate": args.candidate}
    for name in ("gcc", "clang"):
        if getattr(args, name):
            compilers[name] = getattr(args, name)
    env = os.environ.copy()
    env.pop("CCC_DISABLE_PASSES", None)
    versions = {}
    for name, compiler in compilers.items():
        versions[name] = subprocess.check_output(
            [str(compiler.resolve()), "--version"], text=True, env=env
        ).splitlines()[0]
    report = {"cpu": cpu, "trips": args.trips, "samples": args.samples,
              "versions": versions, "kernels": {}}
    with tempfile.TemporaryDirectory(prefix="volatile-reductions-") as work:
        for kernel, source in [
            ("sum", "tls_hot_loop.c"),
            ("sum_and_product", "volatile_reduce.c"),
        ]:
            binaries = {}
            for name, compiler in compilers.items():
                binary = Path(work) / f"{kernel}-{name}"
                subprocess.run([
                    str(compiler.resolve()), "-O3", "-fno-pic", "-no-pie",
                    str(ROOT / "tests/benchmark/programs" / source), "-o", str(binary)
                ], check=True, env=env, timeout=120)
                binaries[name] = binary
            expected = None
            samples = {name: [] for name in binaries}
            names = list(binaries)
            # Two unrecorded warm-up rounds, then rotating-order timed rounds.
            for iteration in range(args.samples + 2):
                offset = iteration % len(names)
                for name in names[offset:] + names[:offset]:
                    before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    start = time.perf_counter()
                    run = subprocess.run([str(binaries[name]), str(args.trips)],
                                         capture_output=True, check=True, timeout=60)
                    wall = time.perf_counter() - start
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    if expected is None:
                        expected = run.stdout
                    if run.stdout != expected:
                        raise RuntimeError(f"{kernel}/{name}: output mismatch")
                    if iteration >= 2:
                        samples[name].append({"wall": wall,
                            "cpu": after.ru_utime + after.ru_stime - before.ru_utime - before.ru_stime})
            medians = {name: {metric: statistics.median(s[metric] for s in values)
                              for metric in ("wall", "cpu")}
                       for name, values in samples.items()}
            report["kernels"][kernel] = {"median_seconds": medians,
                "samples_seconds": samples,
                "wall_speedup": medians["baseline"]["wall"] / medians["candidate"]["wall"]}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({name: {"median_seconds": result["median_seconds"],
                            "wall_speedup": result["wall_speedup"]}
                      for name, result in report["kernels"].items()}, indent=2))


if __name__ == "__main__":
    main()
