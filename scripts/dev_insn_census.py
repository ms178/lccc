#!/usr/bin/env python3
"""Static instruction census of lccc vs GCC/Clang across the benchmark corpus.

Deterministic and exact: counts emitted instructions, and the move fraction.
Used as the *shape* witness for a codegen change; wall time is measured
separately by tests/benchmark/run_benchmarks.py.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path("/home/user/lccc")
PROGS = REPO / "tests/benchmark/programs"
LCCC = REPO / "target/fastbuild/lccc"
GCC = Path("/tmp/tc/opt/gcc-16/bin/gcc-16")
CLANG = Path("/tmp/tc/opt/clang-23.1.4/bin/clang")

INSN = re.compile(r"^\s+[a-z][a-z0-9]*(?:\s|$)")
MOV = re.compile(r"^\s+(mov[a-z]*)\s")


def census(asm: str):
    total = mov = 0
    rip_lea = abs_lea = 0
    for line in asm.splitlines():
        if not INSN.match(line):
            continue
        total += 1
        if MOV.match(line):
            mov += 1
        if line.strip().startswith("leaq") or line.strip().startswith("leal"):
            if "(%rip)" in line:
                rip_lea += 1
            elif re.search(r"leaq?\s+\S+\(%r", line):
                abs_lea += 1
    return total, mov, rip_lea, abs_lea


def compile_asm(cc, src, extra):
    out = subprocess.run(
        [str(cc), "-O2", "-S", *extra, str(src), "-o", "-"],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        return None
    return out.stdout


def main():
    extra = sys.argv[2:] if len(sys.argv) > 2 else []
    rows = []
    for src in sorted(PROGS.glob("*.c")):
        l = compile_asm(LCCC, src, extra)
        if l is None:
            continue
        lt = census(l)
        g = compile_asm(GCC, src, [])
        gt = census(g) if g else (0, 0, 0, 0)
        c = compile_asm(CLANG, src, [])
        ct = census(c) if c else (0, 0, 0, 0)
        rows.append({
            "prog": src.stem,
            "lccc": lt, "gcc": gt, "clang": ct,
        })
    tot = {k: sum(r[k][0] for r in rows) for k in ("lccc", "gcc", "clang")}
    movs = {k: sum(r[k][1] for r in rows) for k in ("lccc", "gcc", "clang")}
    rip = sum(r["lccc"][2] for r in rows)
    absl = sum(r["lccc"][3] for r in rows)
    print(f"{'':8} {'insns':>8} {'moves':>8} {'mov%':>6}")
    for k in ("lccc", "gcc", "clang"):
        print(f"{k:8} {tot[k]:8d} {movs[k]:8d} {100*movs[k]/max(tot[k],1):5.1f}%")
    print(f"\nlccc vs gcc  insns {tot['lccc']/max(tot['gcc'],1):.4f}  "
          f"moves {movs['lccc']/max(movs['gcc'],1):.4f}")
    print(f"lccc vs clang insns {tot['lccc']/max(tot['clang'],1):.4f}  "
          f"moves {movs['lccc']/max(movs['clang'],1):.4f}")
    print(f"lccc leaq rip-relative: {rip}   absolute/base-indexed: {absl}")
    worst = sorted(rows, key=lambda r: -(r["lccc"][0] - r["gcc"][0]))[:10]
    print("\nlargest static instruction excess vs gcc:")
    for r in worst:
        d = r["lccc"][0] - r["gcc"][0]
        print(f"  {r['prog']:26} lccc={r['lccc'][0]:4d} gcc={r['gcc'][0]:4d} "
              f"clang={r['clang'][0]:4d}  +{d}")
    if len(sys.argv) > 1:
        Path(sys.argv[1]).write_text(json.dumps(rows, indent=1))


if __name__ == "__main__":
    main()
