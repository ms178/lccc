#!/usr/bin/env python3
"""Dynamic A/B of one compiler knob, built for a noisy shared VM.

The reference machine is a 2-core VM with neighbours and no PMU. That has one
dominant statistical consequence: **interference is strictly additive**. A
round can be slowed by a co-tenant; it can never be made faster than the
program's true cost. So the sample distribution is the true time plus a
non-negative noise term, and the right estimator of the true time is the
LOWER tail, not the centre.

This script therefore reports the **minimum** of N interleaved rounds as the
primary statistic, with a trimmed mean of the fastest third as a secondary
check. A median — what the first version of this tool used — is biased upward
by exactly the neighbour noise it is supposed to reject, and on this machine
that bias was large enough to invert a verdict.

Other properties that matter here:

* A and B rounds are strictly **interleaved**, so any drift in machine state
  is shared rather than attributed to one side.
* Benchmarks within `--floor-margin` of the measured empty-process floor are
  reported but **excluded from the aggregate**: their ratio is process
  startup, not generated code. (This is the artifact that made `fib` read as
  a 23.8x win when a direct measurement says 149x.)
* **stdout is compared between the two sides.** A mismatch is a hard failure
  and is reported as a miscompile, not as a slow result.
* `--min-delta` suppresses verdicts below a noise threshold so a sub-noise
  wobble is not read as a result.

Usage:
    scripts/perf_ab.py --env CCC_NO_EXPR_SINK=1
    scripts/perf_ab.py --env CCC_FOO=1 --reps 15 --only nbody,expat_xml_scan
"""
from __future__ import annotations

import argparse
import os
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LCCC = os.environ.get("LCCC", str(REPO / "target" / "fastbuild" / "lccc"))
PROGRAMS = REPO / "tests" / "benchmark" / "programs"


def build(src: Path, out: str, extra_env: dict[str, str], flags: list[str]) -> bool:
    env = dict(os.environ)
    env.update(extra_env)
    p = subprocess.run([LCCC, *flags, str(src), "-o", out], capture_output=True,
                       env=env, timeout=900)
    return p.returncode == 0


def run_once(path: str) -> tuple[float, bytes]:
    t0 = time.perf_counter()
    p = subprocess.run([path], capture_output=True, timeout=900)
    return (time.perf_counter() - t0) * 1000.0, p.stdout


def low_mean(xs: list[float]) -> float:
    """Mean of the fastest third — a noise-robust secondary statistic."""
    k = max(1, len(xs) // 3)
    return statistics.fmean(sorted(xs)[:k])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--env", action="append", default=[],
                    help="KEY=VALUE applied to the B side; repeatable")
    ap.add_argument("--opt", default="-O2")
    ap.add_argument("--cflag", action="append", default=[])
    ap.add_argument("--reps", type=int, default=11)
    ap.add_argument("--only", default="")
    ap.add_argument("--floor-margin", type=float, default=1.5)
    ap.add_argument("--min-delta", type=float, default=1.0,
                    help="percent below which a per-benchmark delta is called noise")
    args = ap.parse_args()

    env_b: dict[str, str] = {}
    for kv in args.env:
        k, _, v = kv.partition("=")
        env_b[k] = v
    if not env_b:
        ap.error("--env is required (nothing to A/B)")

    flags = [args.opt, *args.cflag]
    only = {s.strip() for s in args.only.split(",") if s.strip()}
    srcs = sorted(PROGRAMS.glob("*.c"))
    if only:
        srcs = [s for s in srcs if s.stem in only]

    tmp = tempfile.mkdtemp(prefix="perfab-")
    floor_src = Path(tmp) / "floor.c"
    floor_src.write_text(
        "#include <stdio.h>\nint main(void){volatile long r=0;printf(\"%ld\\n\",r);return 0;}\n")
    floor_bin = f"{tmp}/floor"
    subprocess.run(["gcc", "-O2", str(floor_src), "-o", floor_bin], capture_output=True)
    floor = min(run_once(floor_bin)[0] for _ in range(15))

    label = " ".join(f"{k}={v}" for k, v in env_b.items())
    print(f"# perf A/B — A: default   B: {label}")
    print(f"# flags: {' '.join(flags)}   reps: {args.reps}   floor: {floor:.2f} ms")
    print(f"# primary statistic: MINIMUM of {args.reps} interleaved rounds "
          f"(noise on this VM is additive, so the lower tail is the true cost)")
    print()
    print(f"{'benchmark':<26}{'A min':>10}{'B min':>10}{'B/A':>9}{'B/A low3':>10}  note")

    ratios: list[float] = []
    skipped: list[str] = []
    failures: list[str] = []
    movers: list[tuple[float, str]] = []
    for src in srcs:
        a_bin, b_bin = f"{tmp}/{src.stem}.a", f"{tmp}/{src.stem}.b"
        if not build(src, a_bin, {}, flags) or not build(src, b_bin, env_b, flags):
            failures.append(f"{src.stem}: compile")
            continue
        a_t: list[float] = []
        b_t: list[float] = []
        a_out = b_out = None
        try:
            for _ in range(args.reps):
                ta, oa = run_once(a_bin)
                tb, ob = run_once(b_bin)
                a_t.append(ta)
                b_t.append(tb)
                a_out, b_out = oa, ob
        except subprocess.TimeoutExpired:
            failures.append(f"{src.stem}: timeout")
            continue
        if a_out != b_out:
            failures.append(f"{src.stem}: OUTPUT MISMATCH (miscompile)")
            print(f"{src.stem:<26}{'':>10}{'':>10}{'':>9}{'':>10}  *** OUTPUT MISMATCH ***")
            continue
        amin, bmin = min(a_t), min(b_t)
        r = bmin / amin if amin > 0 else 1.0
        r_low = low_mean(b_t) / low_mean(a_t) if low_mean(a_t) > 0 else 1.0
        note = ""
        if amin - floor < args.floor_margin:
            note = "at floor — excluded"
            skipped.append(src.stem)
        else:
            ratios.append(r)
            if abs(r - 1.0) * 100 >= args.min_delta:
                movers.append((r, src.stem))
        print(f"{src.stem:<26}{amin:>10.2f}{bmin:>10.2f}{r:>9.3f}{r_low:>10.3f}  {note}")

    print()
    if ratios:
        g = statistics.geometric_mean(ratios)
        print(f"aggregate over {len(ratios)} above-floor benchmarks: geomean B/A = {g:.4f}")
        pct = (g - 1.0) * 100
        if abs(pct) < args.min_delta:
            print(f"VERDICT: NO MEASURABLE DIFFERENCE ({pct:+.2f}%, below the "
                  f"{args.min_delta:.1f}% noise threshold)")
        elif pct > 0:
            print(f"VERDICT: configuration A is {pct:.2f}% FASTER overall")
        else:
            print(f"VERDICT: configuration A is {-pct:.2f}% SLOWER overall")
        movers.sort()
        if movers:
            print(f"  A faster on: {', '.join(f'{n} {100*(r-1):+.1f}%' for r, n in movers[::-1][:5])}")
            print(f"  A slower on: {', '.join(f'{n} {100*(r-1):+.1f}%' for r, n in movers[:5])}")
    if skipped:
        print(f"excluded (within {args.floor_margin} ms of the {floor:.2f} ms floor): "
              f"{', '.join(skipped)}")
    if failures:
        print("FAILURES: " + "; ".join(failures))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
