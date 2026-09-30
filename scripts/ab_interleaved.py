#!/usr/bin/env python3
"""Interleaved A/B for two lccc binaries on one kernel.

The box this runs on is noisy enough that running arm A to completion and then
arm B lets slow drift masquerade as a speedup. This alternates the two arms
sample by sample, so any drift hits both equally, and reports the MINIMUM of
each arm -- the sample least perturbed by scheduler noise, and the right
statistic for a one-sided distribution.

Also reports a paired win-rate (how often the optimised arm was faster in the
same adjacent pair), which is far more robust than comparing two minima when
the effect is smaller than the run-to-run spread.
"""
import argparse, re, statistics, subprocess, sys, tempfile, os
from pathlib import Path

def build(cc, kern, out, flags, repo):
    r = subprocess.run([cc, *flags, "-w", f"{repo}/tests/bench/driver.c",
                        f"{repo}/tests/bench/k_{kern}.c", "-o", out],
                       capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"build failed for {cc} {kern}: {r.stderr[:400]}")

def sample(binary, cpu):
    r = subprocess.run(["taskset", "-c", cpu, binary], capture_output=True, text=True)
    m = re.search(r"([0-9.]+)\s+(\d+)", r.stdout)
    if not m:
        sys.exit(f"cannot parse driver output: {r.stdout!r}")
    return float(m.group(1)), int(m.group(2))

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--a", required=True); ap.add_argument("--b", required=True)
    ap.add_argument("--kernel", required=True)
    ap.add_argument("--samples", type=int, default=41)
    ap.add_argument("--flags", default="-O3 -march=x86-64-v3")
    ap.add_argument("--cpu", default="0")
    # Default to the repository this script lives in. The old absolute
    # "/home/user/lccc" made the tool fail for every other checkout and
    # every other user -- a measurement harness that only works on the
    # machine that wrote it is not a measurement harness.
    ap.add_argument("--repo", default=str(Path(__file__).resolve().parents[1]))
    a = ap.parse_args()

    flags = a.flags.split()
    ta, tb = [], []
    ta_c = tb_c = None
    wins = 0
    with tempfile.TemporaryDirectory() as d:
        ba, bb = os.path.join(d, "a"), os.path.join(d, "b")
        build(a.a, a.kernel, ba, flags, a.repo)
        build(a.b, a.kernel, bb, flags, a.repo)
        # warm the page cache / branch predictors equally
        for _ in range(3):
            sample(ba, a.cpu); sample(bb, a.cpu)
        for i in range(a.samples):
            if i % 2 == 0:
                ta.append(sample(ba, a.cpu)[0]); tb.append(sample(bb, a.cpu)[0])
            else:
                tb.append(sample(bb, a.cpu)[0]); ta.append(sample(ba, a.cpu)[0])
        # checksums must agree
        _, ta_c = sample(ba, a.cpu); _, tb_c = sample(bb, a.cpu)

    if ta_c != tb_c:
        print(f"CHECKSUM MISMATCH: base={ta_c} opt={tb_c}", file=sys.stderr)
        return 2

    pairs = [(x, y) for x, y in zip(ta, tb)]
    wins = sum(1 for x, y in pairs if y < x)
    loss = sum(1 for x, y in pairs if y > x)
    print(f"kernel            : {a.kernel}   ({a.samples} interleaved samples, {flags})")
    print(f"checksum          : {ta_c} (identical)")
    print(f"base  min/median  : {min(ta)*1000:8.3f} / {statistics.median(ta)*1000:8.3f} ms")
    print(f"opt   min/median  : {min(tb)*1000:8.3f} / {statistics.median(tb)*1000:8.3f} ms")
    print(f"delta on min      : {(min(tb)/min(ta)-1)*100:+.2f} %")
    print(f"delta on median   : {(statistics.median(tb)/statistics.median(ta)-1)*100:+.2f} %")
    print(f"paired win/loss   : {wins} / {loss}  ({wins/len(pairs)*100:.0f}% faster)")
    return 0

if __name__ == "__main__":
    sys.exit(main())
