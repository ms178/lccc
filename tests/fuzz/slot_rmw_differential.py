#!/usr/bin/env python3
"""Differential fuzzer for the i686 slot read-modify-write collapse.

The peephole rewrites `movl S,%R; OP %R; movl %R,S` into a single
memory-destination `OP S` when %R is provably dead afterwards.  The soundness
hinge is the dead-register proof; this fuzzer hammers exactly that hinge with
address-escaped locals (which force slot homes) under both dead-result and
live-result shapes, at O0/O2/Os, comparing lccc's runtime exit fold against
gcc -m32.

Same harness architecture as m32_differential_fuzz.py: self-contained probe,
nostdlib driver, 8-bit folded exit oracle.

Usage:
  python3 slot_rmw_differential.py --ccc path/to/lccc [--seeds 0:200] [--out /tmp/slotrmw]
"""
import argparse
import json
import random
import subprocess
import sys
from pathlib import Path

DRIVER = r"""
.globl _start
_start:
    xorl %ebp, %ebp          # accumulator
    movl $SEEDVAL, %esi      # input seed
    movl $13, %edi           # iterations
1:
    subl $8, %esp
    movl %esi, (%esp)
    movl %edi, 4(%esp)
    call probe
    addl $8, %esp
    xorl %eax, %ebp
    roll $5, %ebp
    imull $0x01000193, %esi, %esi
    addl $0x9e3779b9, %esi
    decl %edi
    jnz 1b
    movl %ebp, %ebx
    andl $255, %ebx
    movl $1, %eax
    int $0x80
"""

PROBE_HEAD = """
typedef int i32; typedef unsigned u32;
__attribute__((noinline)) void escape(i32 *p) { __asm__("" :: "r"(p) : "memory"); }
__attribute__((noinline)) i32 opaque(i32 x) { __asm__("" : "+r"(x)); return x; }
i32 probe(i32 seed, i32 iter) {
"""

OPS = [
    "{v} = {v} + 1;",
    "{v} = {v} - 1;",
    "{v} = {v} + {k};",
    "{v} = {v} - {k};",
    "{v} = {v} & {k};",
    "{v} = {v} | {k};",
    "{v} = {v} ^ {k};",
    "{v} = {v} << ({k}u & 7u);",
    "{v} = -{v};",
    "{v} = ~{v};",
    "{v} += opaque({v}) & 3;",
    "{v} = opaque({v}) + {k};",
]


def gen_probe(seed: int) -> str:
    r = random.Random(seed)
    n_vars = r.randint(1, 4)
    vars_ = [f"v{j}" for j in range(n_vars)]
    lines = [f"  i32 {v} = seed ^ {r.randint(0, 255)} + iter;" for v in vars_]
    for v in vars_:
        if r.random() < 0.85:
            lines.append(f"  escape(&{v});")
    for _ in range(r.randint(3, 10)):
        v = r.choice(vars_)
        op = r.choice(OPS)
        lines.append("  " + op.format(v=v, k=r.randint(1, 31)))
        roll = r.random()
        if roll < 0.25:
            lines.append(f"  escape(&{v});")       # address escapes again
        elif roll < 0.45:
            other = r.choice(vars_)
            lines.append(f"  {other} += {v} & 7;")  # result stays live
        elif roll < 0.55:
            lines.append(f"  seed += {v};")         # live consumer
    fold = " ^ ".join(f"(i32)({v} * {r.randint(1, 31)})" for v in vars_)
    lines.append(f"  return ({fold}) ^ seed;")
    return PROBE_HEAD + "\n".join(lines) + "\n}\n"


def run(cmd, timeout=20):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stderr
    except Exception as e:  # noqa: BLE001
        return "ERR", repr(e)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ccc", required=True)
    ap.add_argument("--gcc", default="/usr/bin/gcc")
    ap.add_argument("--seeds", default="0:200")
    ap.add_argument("--levels", default="O0,O2,Os")
    ap.add_argument("--out", default="/tmp/slotrmw")
    args = ap.parse_args()

    lo, hi = (int(x) for x in args.seeds.split(":"))
    levels = args.levels.split(",")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    results = {"pass": 0, "mismatch": 0, "lccc_compile_failure": 0, "skip": 0}
    failures = []
    for seed in range(lo, hi):
        src = out / f"s{seed}.c"
        src.write_text(gen_probe(seed))
        drv = out / f"d{seed}.s"
        drv.write_text(DRIVER.replace("SEEDVAL", str(0x517cc1b7 + seed * 6829)))
        for level in levels:
            common = ["-m32", "-fno-PIE", "-fomit-frame-pointer", f"-{level}", "-w"]
            lo_obj = out / f"s{seed}_{level}_l.o"
            go_obj = out / f"s{seed}_{level}_g.o"
            rc, err = run([args.ccc, *common, "-fno-pic", "-c", str(src), "-o", str(lo_obj)])
            if rc != 0:
                results["lccc_compile_failure"] += 1
                failures.append({"seed": seed, "level": level, "kind": "ccc_compile", "err": err[-400:]})
                continue
            rc, _ = run([args.gcc, *common, "-c", str(src), "-o", str(go_obj)])
            if rc != 0:
                results["skip"] += 1
                continue
            lbin = out / f"b{seed}_{level}_l"
            gbin = out / f"b{seed}_{level}_g"
            rc1, e1 = run([args.gcc, "-m32", "-nostdlib", "-no-pie", str(drv), str(lo_obj), "-o", str(lbin)])
            rc2, _ = run([args.gcc, "-m32", "-nostdlib", "-no-pie", str(drv), str(go_obj), "-o", str(gbin)])
            if rc1 != 0 or rc2 != 0:
                results["skip"] += 1
                continue
            lrc, _ = run([str(lbin)], timeout=10)
            grc, _ = run([str(gbin)], timeout=10)
            if lrc == grc and isinstance(lrc, int):
                results["pass"] += 1
            else:
                results["mismatch"] += 1
                failures.append({"seed": seed, "level": level, "kind": "mismatch",
                                 "lccc_exit": str(lrc), "gcc_exit": str(grc)})

    summary = {"results": results, "failures": failures[:20],
               "seeds": args.seeds, "levels": levels}
    (out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps(results))
    return 0 if results["mismatch"] == 0 and results["lccc_compile_failure"] == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
