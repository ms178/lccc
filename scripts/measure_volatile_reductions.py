#!/usr/bin/env python3
"""Measure scalar volatile-reduction ILP; never turn noisy timing into a CI gate.

All binaries must agree on stdout and successfully finish. Rotating compiler
order and a pinned CPU reduce, but do not eliminate, virtual-host noise. Pass
explicit GCC 16.2 / Clang 23.1.3 paths for reference comparisons.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
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


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def summary(values):
    median = statistics.median(values)
    return {
        "median": median,
        "mad": statistics.median(abs(v - median) for v in values),
        "min": min(values),
        "max": max(values),
    }


def checked_run(command, *, env, timeout):
    result = subprocess.run(
        command, capture_output=True, check=False, env=env, timeout=timeout
    )
    if result.returncode:
        raise RuntimeError(
            f"child failed ({result.returncode}): {command!r}\n"
            f"stdout: {result.stdout.decode(errors='replace')}\n"
            f"stderr: {result.stderr.decode(errors='replace')}"
        )
    if re.search(rb"\bwarning:", result.stderr, re.I):
        raise RuntimeError(
            f"compiler/runtime warning: {result.stderr.decode(errors='replace')}"
        )
    return result


def provenance(compilers, env):
    tools = {}
    requirements = {
        "gcc": r"\b16\.2(?:\.0)?\b",
        "clang": r"\b23\.1\.3\b",
        "icx": r"\b2026\.2(?:\.0)?\b",
    }
    for name, compiler in compilers.items():
        path = compiler.resolve()
        version = (
            checked_run([str(path), "--version"], env=env, timeout=10)
            .stdout.decode()
            .strip()
        )
        if name in requirements and not re.search(requirements[name], version):
            raise ValueError(f"{name}: wrong reference version: {version}")
        tools[name] = {"path": str(path), "sha256": sha256(path), "version": version}
    cpuinfo = Path("/proc/cpuinfo")
    return {
        "tools": tools,
        "host": platform.uname()._asdict(),
        "cpuinfo": cpuinfo.read_text() if cpuinfo.exists() else None,
        "environment": {
            k: v
            for k, v in env.items()
            if k.startswith(("CCC_", "LCCC_"))
            or k in ("PATH", "LD_LIBRARY_PATH", "OMP_NUM_THREADS")
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--gcc", type=Path)
    parser.add_argument("--clang", type=Path)
    parser.add_argument("--icx", type=Path)
    parser.add_argument("--baseline-revision", default="unspecified")
    parser.add_argument("--candidate-revision", default="unspecified")
    parser.add_argument("--trips", type=positive, default=300000000)
    parser.add_argument("--samples", type=positive, default=9)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    cpu = None
    if hasattr(os, "sched_getaffinity"):
        cpu = min(os.sched_getaffinity(0))
        os.sched_setaffinity(0, {cpu})
    compilers = {"baseline": args.baseline, "candidate": args.candidate}
    for name in ("gcc", "clang", "icx"):
        if getattr(args, name):
            compilers[name] = getattr(args, name)
    env = os.environ.copy()
    env.pop("CCC_DISABLE_PASSES", None)
    report = {
        "cpu": cpu,
        "affinity": sorted(os.sched_getaffinity(0)) if cpu is not None else None,
        "trips": args.trips,
        "samples": args.samples,
        "warmups": 2,
        "provenance": provenance(compilers, env),
        "revisions": {
            "baseline": args.baseline_revision,
            "candidate": args.candidate_revision,
        },
        "kernels": {},
    }
    with tempfile.TemporaryDirectory(prefix="volatile-reductions-") as work:
        for kernel, source in [
            ("sum", "tls_hot_loop.c"),
            ("sum_and_product", "volatile_reduce.c"),
        ]:
            binaries = {}
            builds = {}
            for name, compiler in compilers.items():
                binary = Path(work) / f"{kernel}-{name}"
                command = [
                    str(compiler.resolve()),
                    "-O3",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-fno-pic",
                    "-no-pie",
                    str(ROOT / "tests/benchmark/programs" / source),
                    "-o",
                    str(binary),
                ]
                start = time.perf_counter()
                checked_run(command, env=env, timeout=120)
                builds[name] = {
                    "command": command,
                    "seconds": time.perf_counter() - start,
                    "binary_sha256": sha256(binary),
                }
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
                    run = checked_run(
                        [str(binaries[name]), str(args.trips)], env=env, timeout=60
                    )
                    wall = time.perf_counter() - start
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    if expected is None:
                        expected = run.stdout
                    if run.stdout != expected:
                        raise RuntimeError(f"{kernel}/{name}: output mismatch")
                    if iteration >= 2:
                        samples[name].append(
                            {
                                "wall": wall,
                                "cpu": after.ru_utime
                                + after.ru_stime
                                - before.ru_utime
                                - before.ru_stime,
                            }
                        )
            medians = {
                name: {
                    metric: statistics.median(s[metric] for s in values)
                    for metric in ("wall", "cpu")
                }
                for name, values in samples.items()
            }
            report["kernels"][kernel] = {
                "median_seconds": medians,
                "builds": builds,
                "source_sha256": sha256(ROOT / "tests/benchmark/programs" / source),
                "dispersion": {
                    name: {
                        metric: summary([s[metric] for s in values])
                        for metric in ("wall", "cpu")
                    }
                    for name, values in samples.items()
                },
                "samples_seconds": samples,
                "wall_speedup": medians["baseline"]["wall"]
                / medians["candidate"]["wall"],
            }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(
        json.dumps(
            {
                name: {
                    "median_seconds": result["median_seconds"],
                    "wall_speedup": result["wall_speedup"],
                }
                for name, result in report["kernels"].items()
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
