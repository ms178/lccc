#!/usr/bin/env python3
"""Two-TU count microbenchmarks: alternating paired wall times, never application claims."""
import argparse
import hashlib
import json
import os
import platform
import re
import sys
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SOURCES = [ROOT / "tests/benchmark/programs/count_observations.c", ROOT / "tests/regression/count_observations.c"]


def command(args, env=None):
    p = subprocess.run(args, env=env, capture_output=True, check=True, timeout=120)
    if b"warning:" in p.stderr.lower():
        raise RuntimeError(p.stderr.decode())
    return p.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--compiler", default=str(ROOT / "target/fastbuild/lccc"))
    ap.add_argument("--baseline", required=True)
    ap.add_argument("--gcc", default="gcc-16")
    ap.add_argument("--clang", default="clang-23")
    ap.add_argument("--pairs", type=int, default=15)
    ap.add_argument("--rounds", type=int, default=134217728)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    if args.pairs < 1 or args.rounds < 1:
        ap.error("positive pairs and rounds required")
    report = {"kind": "synthetic isolated calls, two TUs, no LTO", "timing": "wall seconds; two warmups; alternating pair order; no PMU or affinity claim", "rounds": args.rounds, "pairs": args.pairs, "source_sha256": {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in SOURCES}, "cases": {}}
    cpu = Path("/proc/cpuinfo").read_text() if Path("/proc/cpuinfo").exists() else ""
    report["host"] = {"platform": platform.platform(), "cpu_model": next((x.split(":", 1)[1].strip() for x in cpu.splitlines() if x.startswith("model name")), "unknown"), "affinity": sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None}
    report["command"] = sys.argv
    compilers = {"candidate": args.compiler, "baseline": args.baseline, "gcc": args.gcc, "clang": args.clang}
    report["versions"] = {name: command([cc, "--version"]).decode().splitlines()[0] for name, cc in compilers.items()}
    if not re.search(r"\b16\.2(?:\.0)?\b", report["versions"]["gcc"]):
        raise ValueError("GCC 16.2 required")
    if not re.search(r"\b23\.1\.4\b", report["versions"]["clang"]):
        raise ValueError("Clang 23.1.4 required")
    with tempfile.TemporaryDirectory() as tmp:
        for case, name in enumerate(["bounded-popcount", "zero-preimage", "nonzero-implication"]):
            bins = {}
            outputs = {}
            times = {label: [] for label in compilers}
            for label, cc in compilers.items():
                path = str(Path(tmp) / f"{label}-{case}")
                command([cc, "-O2", "-DCOUNT_OBSERVATIONS_EXTERNAL", "-DCOUNT_OBSERVATIONS_KERNEL_ONLY", f"-DCOUNT_CASE={case}", f"-DROUNDS={args.rounds}", *map(str, SOURCES), "-o", path])
                bins[label] = path
                outputs[label] = command([path]).decode().strip()
                command([path])
            if len(set(outputs.values())) != 1:
                raise ValueError(f"checksum mismatch: {outputs}")
            for i in range(args.pairs):
                order = list(bins) if i % 2 == 0 else list(reversed(bins))
                for label in order:
                    start = time.perf_counter()
                    out = command([bins[label]]).decode().strip()
                    times[label].append(time.perf_counter()-start)
                    if out != outputs[label]:
                        raise ValueError("unstable benchmark checksum")
            medians = {label: statistics.median(v) for label, v in times.items()}
            report["cases"][name] = {"checksum": outputs["candidate"], "samples_s": times, "median_s": medians, "baseline_over_candidate": medians["baseline"]/medians["candidate"], "short_sample_warning": min(min(v) for v in times.values()) < .2}
            print(name, medians, flush=True)
    args.output.write_text(json.dumps(report, indent=2)+"\n")


if __name__ == "__main__":
    main()
