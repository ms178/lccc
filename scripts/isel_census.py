#!/usr/bin/env python3
"""Aggregate the CCC_ISEL_STATS MachInst-coverage census over the regression corpus.

Compiles every tests/regression/*.c with the compiler under test at a given
optimization level, parses the per-process [ISEL-STATS] census lines from
stderr, and prints the corpus-wide coverage plus a ranked rejection table.

Usage: CCC_BIN=path/to/lccc [OPT=-O2] [JOBS=N] python3 scripts/isel_census.py
"""
import os, re, subprocess, sys, tempfile, collections
from concurrent.futures import ThreadPoolExecutor

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.environ.get("CCC_BIN", os.path.join(REPO, "target/fastbuild/lccc"))
OPT = os.environ.get("OPT", "-O2")
JOBS = int(os.environ.get("JOBS", "8"))
CORPUS = os.path.join(REPO, "tests/regression")

re_total = re.compile(r"\[ISEL-STATS\] (\d+) of (\d+) instructions lowered through MachInst \(")
re_kind = re.compile(r"\[ISEL-STATS\]\s+rejected\s+(\d+)\s+(\S+)")

def run_one(path):
    try:
        with tempfile.NamedTemporaryFile(suffix=".o", delete=True) as tmp:
            p = subprocess.run([BIN, OPT, "-c", path, "-o", tmp.name],
                               capture_output=True, text=True, timeout=60,
                               env={**os.environ, "CCC_ISEL_STATS": "1"})
        lowered = rejected = 0
        kinds = collections.Counter()
        for line in p.stderr.splitlines():
            m = re_total.search(line)
            if m:
                lowered += int(m.group(1)); rejected += int(m.group(2))
            m = re_kind.search(line)
            if m:
                kinds[m.group(2)] += int(m.group(1))
        return lowered, rejected, kinds
    except Exception:
        return 0, 0, collections.Counter()

def main():
    files = sorted(f for f in os.listdir(CORPUS) if f.endswith(".c"))
    tot_l = tot_r = 0
    kinds = collections.Counter()
    nfail = 0
    with ThreadPoolExecutor(max_workers=JOBS) as ex:
        for l, r, k in ex.map(run_one, [os.path.join(CORPUS, f) for f in files]):
            tot_l += l; tot_r += r; kinds.update(k)
            if l == 0 and r == 0:
                nfail += 1
    tot = tot_l + tot_r
    print(f"corpus: {len(files)} files ({nfail} without census), {tot} instructions")
    print(f"MachInst coverage: {tot_l}/{tot} = {tot_l*100.0/tot:.2f}%")
    print(f"{'rejected':>10}  {'kind':<28} {'% of all':>8}")
    for k, n in kinds.most_common(20):
        print(f"{n:>10}  {k:<28} {n*100.0/tot:>7.2f}%")

if __name__ == "__main__":
    main()
