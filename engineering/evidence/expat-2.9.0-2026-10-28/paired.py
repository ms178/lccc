#!/usr/bin/env python3
"""Paired CPU-clock samples from the unchanged upstream Expat benchmark.

Run after run.sh has built benchmark-{gcc,lccc,clang} and corpus-aligned.xml.
The median of paired ratios uses a percentile bootstrap, not a hardware PMU.
"""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import random
import re
import statistics
import subprocess


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--rounds", type=int, default=40)
    args = parser.parse_args()
    if args.rounds < 2:
        parser.error("at least two paired rounds are required")
    directory = args.directory.resolve(strict=True)
    corpus = directory / "corpus-aligned.xml"
    benchmarks = {name: directory / ("benchmark-" + name) for name in ("gcc", "lccc", "clang")}
    rng = random.Random(20261028)
    loops, buffer_size = 5, 8192
    if corpus.stat().st_size % buffer_size:
        raise ValueError("corpus length must be a multiple of the buffer size")

    def sample(name):
        result = subprocess.run(
            [str(benchmarks[name]), "-n", str(corpus), str(buffer_size), str(loops)],
            text=True, capture_output=True, timeout=30, check=True,
        )
        match = re.search(r"Average time per loop:\s+([0-9.]+)", result.stdout)
        if not match or float(match.group(1)) <= 0:
            raise ValueError("invalid benchmark output: " + result.stdout)
        return float(match.group(1))

    for _ in range(2):
        for name in benchmarks:
            sample(name)
    rows = []
    for _ in range(args.rounds):
        order = list(benchmarks)
        rng.shuffle(order)
        rows.append({name: sample(name) for name in order})

    def summary(numerator):
        ratios = [row[numerator] / row["gcc"] for row in rows]
        boots = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(10000))
        return {
            "median_ratio": statistics.median(ratios),
            "bootstrap_95_pct": [boots[249], boots[9749]],
            "samples": len(ratios),
        }

    result = {
        "clock": "upstream clock(), CPU seconds per parse loop",
        "machine": platform.uname()._asdict(),
        "rounds": rows,
        "summary": {name + "_over_gcc": summary(name) for name in ("lccc", "clang")},
        "seed": 20261028,
        "buffer": buffer_size,
        "loops": loops,
        "corpus_sha256": sha256(corpus),
        **{name + "_benchmark_sha256": sha256(path) for name, path in benchmarks.items()},
    }
    (directory / "paired-benchmark.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result["summary"], indent=2))


if __name__ == "__main__":
    main()
