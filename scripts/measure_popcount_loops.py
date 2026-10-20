#!/usr/bin/env python3
"""Reproducible host popcount density witness, with independent checksums.

Example: --cc target/fastbuild/lccc --cc /path/to/gcc-16 --cc /path/to/clang
          --json work/evidence/popcount-timings.json

Every compiler uses identical explicit ISA flags. The additional LCCC
loop_idiom-disabled binary is a causal control, not another compiler oracle.
Reported process timings include startup and call overhead; they are not an
instruction latency measurement or a claim about unrelated workloads.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "tests/benchmark/programs/popcount_loop_density.c"


def checked(command, **kwargs):
    p = subprocess.run(list(map(str, command)), capture_output=True, text=True, timeout=120, **kwargs)
    if p.returncode or p.stderr.strip():
        raise RuntimeError((command, p.returncode, p.stdout, p.stderr))
    return p.stdout


def expected():
    state = 1234567
    mask = (1 << 64) - 1
    total = 0
    for _ in range(4096):
        state = (state ^ (state << 13)) & mask
        state ^= state >> 7
        state = (state ^ (state << 17)) & mask
        total += state.bit_count()
    return [total * 8192, 4096 * 8192, 63 * 4096 * 8192]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cc", action="append", required=True, help="explicit compiler path (repeatable)")
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    if args.runs < 3:
        parser.error("use at least three runs to report a distribution")
    flags = ["-O2", "-march=x86-64-v3", "-Wall", "-Wextra", "-Werror"]
    report = {"host": platform.platform(), "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
              "flags": flags, "runs": args.runs, "results": []}
    cpuinfo = Path("/proc/cpuinfo")
    if cpuinfo.exists():
        report["cpu"] = next((line.split(":", 1)[1].strip() for line in cpuinfo.read_text().splitlines()
                              if line.startswith("model name")), "unknown")
    sums = expected()
    with tempfile.TemporaryDirectory(prefix="lccc-density-") as directory:
        w = Path(directory)
        for index, name in enumerate(args.cc):
            cc = Path(name).resolve(strict=True)
            version = checked([cc, "--version"]).splitlines()[0]
            for control in ([False, True] if cc.name in ("lccc", "lccc-x86") else [False]):
                binary = w / f"cc-{index}-{int(control)}"
                env = dict(os.environ)
                if control:
                    env["CCC_DISABLE_PASSES"] = "loop_idiom"
                else:
                    env.pop("CCC_DISABLE_PASSES", None)
                checked([cc, *flags, SOURCE, "-o", binary], env=env)
                record = {"compiler": str(cc), "version": version, "loop_idiom_disabled": control,
                          "compiler_sha256": hashlib.sha256(cc.read_bytes()).hexdigest(), "modes": {}}
                for mode, label in enumerate(("random", "sparse", "dense")):
                    output = checked([binary, mode])
                    if int(output) != sums[mode]:
                        raise RuntimeError((label, "checksum", output, sums[mode]))
                    samples = []
                    for _ in range(args.runs):
                        start = time.perf_counter_ns()
                        output = checked([binary, mode])
                        samples.append((time.perf_counter_ns() - start) / 1e9)
                        if int(output) != sums[mode]:
                            raise RuntimeError((label, "unstable checksum", output))
                    record["modes"][label] = {"checksum": sums[mode], "seconds": samples,
                        "median_seconds": statistics.median(samples), "min_seconds": min(samples),
                        "max_seconds": max(samples)}
                report["results"].append(record)
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
