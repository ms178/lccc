#!/usr/bin/env python3
"""Deterministic Callgrind A/B for the benchmark corpus.

Wall-timed A/B on a shared 2-vCPU VM is noisy; Callgrind gives
reproducible simulated instruction counts (Ir), I1/LL instruction-cache
misses and branch mispredictions for the SAME binary on every run.
Instruction counts decide uop-level regressions; I1/LLi misses decide
the frontend placement effects (loop/function alignment) that wall time
only hints at.

Usage:
  scripts/callgrind_ab.py MINE_LCCC REF_LCCC OPT [bench...]

Writes /tmp/cg_<opt>/{mine,ref}/<bench>.* and prints a markdown table.
"""
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
INCLUDE = "-I/usr/lib/gcc/x86_64-linux-gnu/14/include"

# Fast/medium corpus; heavy multi-second drivers are opt-in (they take
# minutes each under ~30x Callgrind instrumentation).
DEFAULT_FAST = [
    "arith_loop", "fib", "matmul", "sieve", "tce_sum", "spectral_norm",
    "switch_dispatch", "struct_copy", "loop_patterns", "ackermann",
    "constant_recursion", "bitops", "double_reduction", "ascii_case_fold",
    "binary_search", "ring_fifo", "histogram", "gzip_crc32",
    "libm_round_family", "tls_seg_access", "zlib_ng_adler32",
    "expat_xml_scan", "sqlite_varint", "linux_find_bit", "glibc_memcmp",
    "chacha20_block", "sha256_transform", "linux_rbtree", "zstd_count",
    "lz4_compress", "qsort",
]

HEAVY = {"nbody", "mandelbrot", "hash_table", "strlen_bench", "fannkuch",
         "binary_trees", "glibc_strstr"}


def compile(lccc, opt, src, out):
    r = subprocess.run([str(lccc), INCLUDE, opt, "-o", str(out), str(src)],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return r.stderr[-300:]
    return None


def callgrind(binpath, outdir):
    cg = outdir / (binpath.name + ".cg")
    env = dict(os.environ)
    p = subprocess.run(
        ["valgrind", "--tool=callgrind", "--cache-sim=yes",
         "--branch-sim=yes", "--quiet",
         f"--callgrind-out-file={cg}", str(binpath)],
        capture_output=True, text=True, env=env, timeout=1200)
    if p.returncode != 0:
        return None, p.stderr[-400:]
    return parse_summary(cg), None


def parse_summary(cg: Path):
    """Parse the 'summary:' totals line from a callgrind.out file."""
    if not cg.exists():
        return None
    events, totals = [], []
    for line in cg.read_text(errors="replace").splitlines():
        if line.startswith("events:"):
            events = line.split()[1:]
        elif line.startswith("summary:"):
            totals = [int(x) for x in line.split()[1:]]
    return dict(zip(events, totals))


def main():
    mine, ref, opt = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    benches = sys.argv[4:] or DEFAULT_FAST
    outroot = Path(f"/tmp/cg_{opt.lstrip('-')}")
    (outroot / "mine").mkdir(parents=True, exist_ok=True)
    (outroot / "ref").mkdir(parents=True, exist_ok=True)
    rows = []
    for b in benches:
        src = REPO / "tests/benchmark/programs" / f"{b}.c"
        if not src.exists():
            print(f"{b}: missing source"); continue
        m_bin, r_bin = outroot / "mine" / b, outroot / "ref" / b
        e1 = compile(mine, opt, src, m_bin)
        if e1:
            print(f"{b}: MINE compile failed: {e1.strip()[:120]}"); continue
        e2 = compile(ref, opt, src, r_bin)
        if e2:
            print(f"{b}: REF compile failed: {e2.strip()[:120]}"); continue
        # correctness
        om = subprocess.run([m_bin], capture_output=True).stdout
        or_ = subprocess.run([r_bin], capture_output=True).stdout
        if om != or_:
            print(f"{b}: OUTPUT MISMATCH"); continue
        m_ev, e3 = callgrind(m_bin, outroot / "mine")
        r_ev, e4 = callgrind(r_bin, outroot / "ref")
        if not m_ev or not r_ev:
            print(f"{b}: callgrind failed: {(e3 or e4)[:100]}"); continue
        rows.append((b, r_ev, m_ev))
    print("\n| benchmark | Ir ref | Ir mine | Ir m/r | I1miss m/r | IL miss m/r | Bcmisp m/r | Bimisp m/r |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    agg = {}
    for b, r, m in rows:
        def g(ev, *names):
            for n in names:
                if n in ev:
                    return ev[n]
            return 0
        irr, irm = g(r, "Ir"), g(m, "Ir")
        i1r, i1m = g(r, "I1mr"), g(m, "I1mr")
        llr, llm = g(r, "ILmr"), g(m, "ILmr")
        bmr, bmm = g(r, "Bcm"), g(m, "Bcm")
        bir, bim = g(r, "Bim"), g(m, "Bim")
        print(f"| {b} | {irr:,} | {irm:,} | {irm/irr:.5f} | "
              f"{i1m}/{i1r} | {llm}/{llr} | {bmm}/{bmr} | {bim}/{bir} |")
        agg.setdefault("ir", []).append(irm / irr)
    import math
    if agg["ir"]:
        print(f"\ngeomean Ir mine/ref = "
              f"{math.exp(sum(math.log(x) for x in agg['ir'])/len(agg['ir'])):.5f}")


if __name__ == "__main__":
    main()
