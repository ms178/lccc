#!/usr/bin/env python3
"""Measure the dual-layout metric pair split, reproducibly.

The pair scheduler is a *latency* change, so its evidence has to be timing
evidence plus an output-identity control. This script produces both from one
run and writes them as JSON, so the numbers in the journal can be regenerated
instead of trusted:

* interleaved A/B — every measured pair runs serial and heuristic-parallel
  back to back, so a drifting host penalises both arms equally;
* randomised arm order — the arms of one iteration are shuffled from a seeded
  RNG. A fixed order biased one arm: the same TU measured 1.021 in a batch and
  0.871-0.912 alone, minutes apart, purely from frequency drift and page-cache
  warming. The seed is recorded, so the run stays reproducible;
* warm-up discarded, median over the samples, percentile-bootstrap interval
  (seeded, so the interval is reproducible);
* every run's assembly is hashed: the split may not change output by one byte;
* the compiler's own `[pair]` telemetry is captured, so the report states how
  many pairs were actually split rather than assuming the gate fired.

Usage:
  python3 scripts/measure_peephole_pair.py --json engineering/evidence/....json
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import re
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SOURCES = [
    "tests/benchmark/programs/expat_siphash24.c",
    "tests/benchmark/programs/zstd_count.c",
    "tests/benchmark/programs/linux_rbtree.c",
]
# Format-tolerant: the telemetry line carries an `arch=` field, and the
# script must keep parsing if more fields are added later.
PAIR = re.compile(r"\[pair\][^\n]*?chain=(\d+)B orig=(\d+)B mode=(\w+)")


def run_once(compiler, source, knob, out):
    env = dict(os.environ)
    env.pop("LCCC_PEEPHOLE_PAIR_PARALLEL", None)
    if knob is not None:
        env["LCCC_PEEPHOLE_PAIR_PARALLEL"] = knob
    env["CCC_TIME_PEEPHOLE"] = "1"
    started = time.perf_counter()
    result = subprocess.run(
        [str(compiler), "-O1", "-S", str(source), "-o", str(out)],
        capture_output=True, text=True, env=env,
    )
    elapsed = time.perf_counter() - started
    if result.returncode != 0:
        raise ValueError(f"{source.name}: {result.stderr.strip()[:400]}")
    digest = hashlib.sha256(Path(out).read_bytes()).hexdigest()
    pairs = [(int(a), int(b), mode) for a, b, mode in PAIR.findall(result.stderr)]
    return elapsed, digest, pairs


def bootstrap_ci(samples, resamples=20000, seed=20261021):
    rng = random.Random(seed)
    n = len(samples)
    medians = []
    for _ in range(resamples):
        medians.append(statistics.median(rng.choices(samples, k=n)))
    medians.sort()
    return medians[int(0.025 * resamples)], medians[int(0.975 * resamples) - 1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--compiler", default=os.environ.get("CCC", str(ROOT / "target/fastbuild/lccc"))
    )
    parser.add_argument("--source", action="append", help="repeatable; defaults to a mixed set")
    parser.add_argument("--iters", type=int, default=15)
    parser.add_argument("--seed", type=int, default=20261022,
                        help="seed for the per-iteration arm shuffle")
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument(
        "--arms", default="serial,auto",
        help="comma list of scheduling arms to interleave; "
             "'serial' forces =0, 'auto' leaves the heuristic, 'forced' forces =1",
    )
    parser.add_argument("--json", help="write the raw records here")
    args = parser.parse_args()
    knobs = {"serial": "0", "auto": None, "forced": "1"}
    arms = [arm.strip() for arm in args.arms.split(",") if arm.strip()]
    if "serial" not in arms:
        raise ValueError("the 'serial' arm is the baseline every ratio divides by")

    compiler = Path(args.compiler)
    sources = [Path(s) if Path(s).is_absolute() else ROOT / s
               for s in (args.source or DEFAULT_SOURCES)]
    if not compiler.exists():
        raise ValueError(f"compiler not found: {compiler}")

    report = {"compiler": str(compiler), "iters": args.iters, "warmups": args.warmups,
              "arms": arms, "arm_order": "seeded per-iteration shuffle",
              "seed": args.seed, "sources": {}}
    overall_identity = True
    with tempfile.TemporaryDirectory() as temp:
        out = Path(temp) / "tu.s"
        for source in sources:
            if not source.is_file():
                raise ValueError(f"missing source: {source}")
            for _ in range(args.warmups):
                for arm in arms:
                    run_once(compiler, source, knobs[arm], out)

            times = {arm: [] for arm in arms}
            digests = set()
            per_arm_telemetry = {}
            # Randomised arm order per iteration (seeded, so the record is
            # reproducible). A fixed order lets CPU frequency drift and page
            # cache warming bias one arm: measuring a single TU gave 1.021 in
            # a fixed-order batch and 0.871-0.912 alone, a swing far larger
            # than the effect under test.
            rng = random.Random(args.seed)
            for _ in range(args.iters):
                order = arms[:]
                rng.shuffle(order)
                for arm in order:
                    elapsed, digest, pairs = run_once(compiler, source, knobs[arm], out)
                    times[arm].append(elapsed)
                    digests.add(digest)
                    per_arm_telemetry[arm] = pairs

            identical = len(digests) == 1
            overall_identity &= identical
            base = statistics.median(times["serial"])
            rows = {}
            for arm in arms:
                ratios = [t / s for t, s in zip(times[arm], times["serial"])]
                low, high = bootstrap_ci(ratios)
                telemetry = per_arm_telemetry[arm]
                rows[arm] = {
                    "median_s": statistics.median(times[arm]),
                    "ratio_median": statistics.median(ratios),
                    "ratio_ci95": [low, high],
                    "ratios": ratios,
                    "pairs": [{"chain_bytes": a, "orig_bytes": b, "mode": m}
                              for a, b, m in telemetry],
                    "pairs_split": sum(row[2] == "parallel" for row in telemetry),
                }
                print(
                    f"{source.name} [{arm}]: {statistics.median(ratios):.4f} "
                    f"[{low:.4f}, {high:.4f}] "
                    f"({statistics.median(times[arm]) * 1e3:.2f} ms vs {base * 1e3:.2f} ms) "
                    f"pairs={len(telemetry)} split={rows[arm]['pairs_split']}"
                )
            report["sources"][source.name] = {
                "assembly_sha256": sorted(digests),
                "assembly_identical": identical,
                "arms": rows,
            }

    if not overall_identity:
        raise ValueError("scheduling mode changed the emitted assembly: latency-only contract broken")
    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
