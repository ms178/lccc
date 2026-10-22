#!/usr/bin/env python3
"""Rebalance the benchmark lane's shards from a measured corpus run.

The corpus runs in shards so the lane's wall clock is the slowest shard rather
than the sum of every kernel.  The shard lists are checked into `ci.yml` (a
workflow that computes its own inputs is unreviewable, and a wrong split is
invisible in a green run), so this tool exists to REGENERATE them and to show
the reasoning, not to run inside CI.

Cost model, per kernel on the hosted runner:

    10 x (timed wall of one round-set)  +  fixed per-kernel cost

The 10 is `--reps 9 --warmup 1`: every kernel is executed ten times per
compiler.  The fixed part covers two compiles and twenty process spawns, which
dominate for the small kernels -- 466 s of hosted corpus time over 49 kernels
is ~9.5 s each, while most kernels' timed work is under a second. A pure
timed-work split would therefore hand one shard twenty tiny kernels and the
other the heavy ones; the constant keeps the COUNT balanced too.

    scripts/ci_bench_shards.py --report bench-probe.json --shards 2

The probe is any report produced with `--reps 1 --warmup 0`, which is cheap
enough to re-take whenever the registry changes:

    python3 .github/scripts/ci-bench.py --lccc target/fastbuild/lccc \\
        --compilers lccc,gcc --gcc gcc-16 --reps 1 --warmup 0 --cpu none \\
        --json bench-probe.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

FIXED_SECONDS = 6.0
ROUNDS = 10


def kernel_weights(report: dict) -> dict[str, float]:
    weights: dict[str, float] = {}
    for entry in report["benchmarks"]:
        timed = sum(run["wall_s"]
                    for runs in entry.get("runs", {}).values()
                    for run in runs if run.get("ok"))
        weights[entry["name"]] = ROUNDS * timed + FIXED_SECONDS
    return weights


def lpt(weights: dict[str, float], shards: int) -> tuple[dict[str, int], list[float]]:
    totals = [0.0] * shards
    assignment: dict[str, int] = {}
    for name in sorted(weights, key=lambda n: (-weights[n], n)):
        target = min(range(shards), key=lambda s: (totals[s], s))
        assignment[name] = target
        totals[target] += weights[name]
    return assignment, totals


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", type=Path, required=True,
                        help="probe report from --reps 1 --warmup 0")
    parser.add_argument("--shards", type=int, default=2)
    parser.add_argument("--emit-yaml", action="store_true",
                        help="print the `only:` blocks for ci.yml")
    args = parser.parse_args()

    if not args.report.is_file():
        print(f"missing probe report {args.report}", file=sys.stderr)
        return 2
    report = json.loads(args.report.read_text())
    if not report.get("complete"):
        print(f"{args.report}: the probe did not complete; re-run it", file=sys.stderr)
        return 2
    weights = kernel_weights(report)
    assignment, totals = lpt(weights, args.shards)

    print(f"{len(weights)} kernels, model total {sum(totals):.0f} s")
    for shard in range(args.shards):
        names = sorted(n for n in weights if assignment[n] == shard)
        spread = " ".join(f"{n}={weights[n]:.0f}s" for n in
                          sorted(names, key=lambda n: -weights[n])[:4])
        print(f"\nshard {shard}: {len(names)} kernels, {totals[shard]:.0f} s predicted")
        print(f"  heaviest: {spread}")
        if args.emit_yaml:
            print("      only: >-")
            for name in names:
                print(f"          {name},")
        else:
            print("  " + ", ".join(names))
    mean = sum(totals) / len(totals)
    spread = max(totals) - min(totals)
    print(f"\nimbalance: {spread:.0f} s ({100 * spread / mean:.1f} % of the mean)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
