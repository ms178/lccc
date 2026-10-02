#!/usr/bin/env python3
"""Deterministic Callgrind A/B for the benchmark corpus.

Wall-timed A/B on a shared 2-vCPU VM is noisy; Callgrind gives
reproducible simulated instruction counts (Ir), I1/LL instruction-cache
misses and branch mispredictions for the SAME binary on every run.
Instruction counts decide uop-level regressions; I1/LLi misses decide
the frontend placement effects (loop/function alignment) that wall time
only hints at.

Usage:
  scripts/callgrind_ab.py MINE_LCCC REF_LCCC "OPT [OPT...]" [bench...]

Writes /tmp/cg_<opt-slug>/{aa,bb}/<bench>.* and prints a markdown table.

ISA MATCHING IS THE CALLER'S JOB, and the script refuses to guess: lccc's
default target enables AVX2 while gcc's defaults to baseline x86-64 (SSE2), so
an A/B run as `lccc -O2` vs `gcc -O2` measures the ISA, not the compiler.  Pass
the architecture in OPT (e.g. `-O2 -march=x86-64-v3`) and it is handed to BOTH
compilers verbatim.  Why this is spelled out here: an earlier round of this
script's own results (matmul 0.29x, double_reduction 0.29x) were recorded
without the flag and overstated lccc by the width of the vector unit.
"""
import functools
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]


@functools.lru_cache(maxsize=1)
def gcc_include() -> str:
    """Resolved freestanding include path (never a hardcoded gcc version)."""
    out = subprocess.run(["gcc", "-print-file-name=include"],
                         capture_output=True, text=True, check=True).stdout.strip()
    return f"-I{out}"


INCLUDE = gcc_include()

# ---------------------------------------------------------------------------
# Fixed Callgrind/Cachegrind cache geometry — DO NOT autodiscover.
#
# Valgrind's cache simulation otherwise probes the host CPU, so the same
# binary reports different I1/LL miss counts on every machine/restore.  We pin
# a mainstream desktop geometry (32 KiB 8-way 64 B-line L1i/L1d, 32 MiB
# 16-way LLC) so paired A/B ratios and absolute counts are comparable across
# sessions and hosts.  Callgrind models I1, D1 and LL only (no L2) — that is a
# tool limit, not an oversight.  Same doctrine as the EDG front end's
# benchmark harness (edgcpp/compiler `dev_tools/bin/edg-bench`), which pins
# this exact geometry for the same reason.  Overrides exist for deliberate
# what-if runs but MUST be recorded in the artifact manifest if used.
# ---------------------------------------------------------------------------
CG_I1 = os.environ.get("LCCC_CG_I1", "32768,8,64")
CG_D1 = os.environ.get("LCCC_CG_D1", "32768,8,64")
CG_LL = os.environ.get("LCCC_CG_LL", "33554432,16,64")

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
    opts = opt.split() if isinstance(opt, str) else list(opt)
    r = subprocess.run([str(lccc), INCLUDE, *opts, "-o", str(out), str(src)],
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
         # FIXED cache geometry (see CG_* below): Valgrind otherwise
         # autodiscovers the host caches and the simulated miss counts stop
         # being comparable across hosts/restores.
         f"--I1={CG_I1}", f"--D1={CG_D1}", f"--LL={CG_LL}",
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


def compiler_version(cmd: Path) -> str:
    try:
        return subprocess.run([str(cmd), "--version"], capture_output=True,
                              text=True, timeout=30).stdout.splitlines()[0]
    except (OSError, subprocess.SubprocessError, IndexError):
        return "unknown"


def main():
    mine, ref, opt = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
    benches = sys.argv[4:] or DEFAULT_FAST
    tag = "_".join(tok.lstrip("-") for tok in opt.split())
    outroot = Path(f"/tmp/cg_{tag}")
    # NOTE: the two sides MUST live on equal-length paths.  Valgrind counts
    # instructions executed inside ld.so/glibc startup, whose string loops are
    # path-length dependent; a 1-character asymmetry between the two sides
    # injects a deterministic +/-14 Ir artifact (measured, e.g. fib: path
    # length 17 -> 118053 vs 14/15/16 -> 118067).  Hence "aa"/"bb", not
    # "mine"/"ref".
    mdir, rdir = outroot / "aa", outroot / "bb"
    mdir.mkdir(parents=True, exist_ok=True)
    rdir.mkdir(parents=True, exist_ok=True)
    # Record the pinned geometry up front: an A/B without its cache model is
    # not reproducible evidence.  Overrides are visible, never silent.
    env_overrides = {k: os.environ[k] for k in
                     ("LCCC_CG_I1", "LCCC_CG_D1", "LCCC_CG_LL") if k in os.environ}
    print(f"# callgrind geometry: I1={CG_I1} D1={CG_D1} LL={CG_LL}"
          + (f"  [env overrides: {env_overrides}]" if env_overrides else ""))
    manifest = {
        "geometry": {"I1": CG_I1, "D1": CG_D1, "LL": CG_LL},
        "geometry_env_overrides": env_overrides,
        "opt": opt,
        "mine": {"path": str(mine), "version": compiler_version(mine)},
        "ref": {"path": str(ref), "version": compiler_version(ref)},
        "tool": "valgrind callgrind --cache-sim=yes --branch-sim=yes",
        "results": {},
    }
    rows = []
    failures = {}
    for b in benches:
        src = REPO / "tests/benchmark/programs" / f"{b}.c"
        if not src.exists():
            print(f"{b}: missing source")
            failures[b] = "missing-source"
            continue
        m_bin, r_bin = mdir / b, rdir / b
        e1 = compile(mine, opt, src, m_bin)
        if e1:
            print(f"{b}: MINE compile failed: {e1.strip()[:120]}")
            failures[b] = f"mine-compile: {e1.strip()[:120]}"
            continue
        e2 = compile(ref, opt, src, r_bin)
        if e2:
            print(f"{b}: REF compile failed: {e2.strip()[:120]}")
            failures[b] = f"ref-compile: {e2.strip()[:120]}"
            continue
        # Correctness: exit status AND stdout (a crash printing the right
        # bytes is still wrong — review P2 inherited debt, fixed here).
        pm = subprocess.run([m_bin], capture_output=True)
        pr = subprocess.run([r_bin], capture_output=True)
        if pm.returncode != 0 or pr.returncode != 0 or pm.stdout != pr.stdout:
            print(f"{b}: OUTPUT MISMATCH (rc mine/ref "
                  f"{pm.returncode}/{pr.returncode})")
            failures[b] = f"output-mismatch rc {pm.returncode}/{pr.returncode}"
            continue
        m_ev, e3 = callgrind(m_bin, mdir)
        r_ev, e4 = callgrind(r_bin, rdir)
        if not m_ev or not r_ev:
            print(f"{b}: callgrind failed: {(e3 or e4)[:100]}")
            failures[b] = f"callgrind: {(e3 or e4)[:100]}"
            continue
        rows.append((b, r_ev, m_ev))
        manifest["results"][b] = {"ref": r_ev, "mine": m_ev}
    print("\n| benchmark | Ir ref | Ir mine | Ir m/r | I1miss m/r | D1miss r+w m/r |"
          " LLmiss r+w m/r | Bcmisp m/r | Bimisp m/r |")
    print("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    agg = {}
    for b, r, m in rows:
        def g(ev, *names):
            for n in names:
                if n in ev:
                    return ev[n]
            return 0
        irr, irm = g(r, "Ir"), g(m, "Ir")
        i1r, i1m = g(r, "I1mr"), g(m, "I1mr")
        # D1 misses INCLUDE write misses: a store-heavy kernel must not look
        # cheaper than it is (review P2).
        d1r = g(r, "D1mr") + g(r, "D1mw")
        d1m = g(m, "D1mr") + g(m, "D1mw")
        llr, llm = g(r, "ILmr") + g(r, "DLmr") + g(r, "DLmw"), \
            g(m, "ILmr") + g(m, "DLmr") + g(m, "DLmw")
        bmr, bmm = g(r, "Bcm"), g(m, "Bcm")
        bir, bim = g(r, "Bim"), g(m, "Bim")
        print(f"| {b} | {irr:,} | {irm:,} | {irm/irr:.5f} | "
              f"{i1m}/{i1r} | {d1m}/{d1r} | {llm}/{llr} | {bmm}/{bmr} | {bim}/{bir} |")
        agg.setdefault("ir", []).append(irm / irr)
    import math
    if agg.get("ir"):
        geo = math.exp(sum(math.log(x) for x in agg["ir"]) / len(agg["ir"]))
        print(f"\ngeomean Ir mine/ref = {geo:.5f}")
        manifest["geomean_ir_mine_over_ref"] = round(geo, 6)
    manifest["failures"] = failures
    (outroot / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"manifest -> {outroot / 'manifest.json'}")
    if not rows:
        print("error: no benchmark produced comparable events — refusing to "
              "call this a successful run", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main() or 0)
