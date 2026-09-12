#!/usr/bin/env python3
"""Alignment-policy A/B study on a hot-benchmark subset.

Compiles each benchmark with one LCCC binary under several GCC-compatible
alignment policies (default, -falign-functions=32/64, plus loop variants),
links, verifies outputs match, then times the variants INTERLEAVED (each
variant appears in every rotation slot equally) and reports median ratios
per benchmark and an aggregate geometric mean.

Pure flags experiment: isolates the alignment policy from every other
codegen difference. Usage:

  scripts/align_policy_ab.py LCCC OPT [RUNS] [bench...]
"""
import itertools
import statistics as st
import subprocess
import sys
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]

VARIANTS = {
    "default": [],
    "func32": ["-falign-functions=32"],
    "func64": ["-falign-functions=64"],
    "func32_loop32": ["-falign-functions=32", "-falign-loops=32"],
}

# Hot, loop-heavy subset (driver self-times in well under ~1 s each on the
# study box except where noted).
DEFAULT_BENCH = [
    "arith_loop", "gzip_crc32", "zlib_ng_adler32", "expat_xml_scan",
    "sqlite_varint", "linux_find_bit", "glibc_memcmp", "chacha20_block",
    "sha256_transform", "linux_rbtree", "zstd_count", "lz4_compress",
    "strlen_bench", "switch_dispatch", "sieve", "histogram", "ring_fifo",
    "ascii_case_fold", "loop_patterns", "double_reduction", "matmul",
    "fib", "bitops",
]


def compile_run(lccc, opt, flags, src, out):
    cmd = [str(lccc), opt, *flags, "-o", str(out), str(src)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return r.stderr[-300:]
    return None


def main():
    lccc = Path(sys.argv[1]); opt = sys.argv[2]
    runs = int(sys.argv[3]) if len(sys.argv) > 3 else 15
    bench = sys.argv[4:] or DEFAULT_BENCH
    results = {v: [] for v in VARIANTS}
    td = Path(tempfile.mkdtemp(dir="/tmp", prefix="alignab_"))
    pats = list(itertools.permutations(VARIANTS))
    pats = pats[:6]
    for name in bench:
        src = REPO / "tests/benchmark/programs" / f"{name}.c"
        bins = {}
        outs = {}
        ok = True
        for v, fl in VARIANTS.items():
            out = td / f"{name}_{v}"
            err = compile_run(lccc, opt, fl, src, out)
            if err is not None:
                print(f"{name:24s} {v}: compile failed: {err.strip()[:120]}")
                ok = False
                break
            bins[v] = out
            outs[v] = subprocess.run([out], capture_output=True).stdout
        if not ok:
            continue
        if not all(o == outs["default"] for o in outs.values()):
            print(f"{name:24s} OUTPUT MISMATCH - skipping")
            continue
        for v in bins:
            for _ in range(3):
                subprocess.run([bins[v]], stdout=subprocess.DEVNULL)
        times = {v: [] for v in bins}
        cyc = itertools.cycle(pats)
        for _ in range(runs):
            for v in next(cyc):
                t0 = time.perf_counter()
                subprocess.run([bins[v]], stdout=subprocess.DEVNULL)
                times[v].append(time.perf_counter() - t0)
        md = {v: st.median(xs) for v, xs in times.items()}
        line = f"{name:24s} " + " ".join(
            f"{v}={md[v]/md['default']:.4f}" for v in VARIANTS if v != "default")
        print(line, flush=True)
        for v in VARIANTS:
            if v != "default":
                results[v].append((name, md[v] / md["default"]))
    print("\n== aggregate geometric mean (default=1.0; <1 faster) ==")
    import math
    for v, rs in results.items():
        if rs:
            g = math.exp(sum(math.log(r) for _, r in rs) / len(rs))
            wins = sum(1 for _, r in rs if r < 0.99)
            loss = sum(1 for _, r in rs if r > 1.01)
            worst = max(rs, key=lambda z: z[1])
            best = min(rs, key=lambda z: z[1])
            print(f"{v:16s} geomean={g:.4f} wins(<.99)={wins} losses(>1.01)={loss} "
                  f"best={best[0]}:{best[1]:.3f} worst={worst[0]}:{worst[1]:.3f}")


if __name__ == "__main__":
    main()
