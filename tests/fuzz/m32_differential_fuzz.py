#!/usr/bin/env python3
"""i686 (-m32) differential fuzz without a 32-bit libc.

Generates deterministic, libc-free C functions (int32 arithmetic, control
flow, arrays, pointer walks -- the constructs the i686 ecx/edx caller-saved
allocation touches), compiles them with lccc -m32 and gcc -m32, links each
against a tiny nostdlib _start driver that folds the results of many calls
into an 8-bit exit code, runs both binaries and compares exit codes.

Usage:
  python3 m32_differential_fuzz.py --ccc path/to/lccc [--seeds 0:100]
                                   [--levels O0,O2,Os] [--out /tmp/m32fz]
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
    movl $17, %edi           # iterations
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


def gen_c(seed: int) -> str:
    rng = random.Random(seed)
    n_vars = rng.randint(4, 9)
    vars_ = [f"v{i}" for i in range(n_vars)]
    lines = [
        "typedef unsigned int u32;",
        "typedef int i32;",
        "static u32 g_arr[16];",
        "static u32 g_x;",
        "static u32 helper(u32 a, u32 b) {",
        "  return (a ^ (b << 3)) + (a >> (b & 15)) + (b | 0x55);",
        "}",
        "i32 probe(i32 seed, i32 iter) {",
    ]
    for i, v in enumerate(vars_):
        expr = rng.choice([
            f"(u32)seed * {rng.randint(3, 977)}u",
            f"(u32)seed ^ {rng.randint(1, 1 << 30)}u",
            f"(u32)(seed + iter * {rng.randint(1, 99)})",
            f"(u32)seed >> {rng.randint(1, 7)}",
        ])
        lines.append(f"  u32 {v} = {expr};")
    lines.append("  g_x = (u32)seed;")
    lines.append(f"  for (i32 i = 0; i < (iter & 15) + 2; i++) {{")
    lines.append(f"    g_arr[i & 15] = g_arr[(i + 3) & 15] + {vars_[0]} + (u32)i;")
    body_ops = rng.randint(4, 10)
    for _ in range(body_ops):
        d = rng.choice(vars_)
        a = rng.choice(vars_)
        b = rng.choice(vars_)
        op = rng.choice([
            f"{d} = {a} + {b};",
            f"{d} = {a} - ({b} >> 2);",
            f"{d} = {a} * ({b} | 1u);",
            f"{d} = {a} ^ ({b} + 0x9e37u);",
            f"{d} = {a} & ({b} | 0x0f0fu);",
            f"{d} = ({a} << ({b} & 7u)) | ({a} >> (32 - ({b} & 7u) - 1));",
            f"{d} = helper({a}, {b});",
            f"{d} = ({a} > {b}) ? {a} - {b} : {b} - {a};",
            f"if ({a} % 3u == 1u) {d} += g_arr[{b} & 15];",
            f"{d} = {a} / (({b} & 7u) + 1u);",
            f"{d} = {a} % (({b} & 15u) + 1u);",
        ])
        lines.append(f"    {op}")
    lines.append("    g_x = g_x * 1664525u + 1013904223u;")
    lines.append("  }")
    fold = " ^ ".join(vars_)
    lines.append(f"  return (i32)({fold} ^ g_x ^ g_arr[seed & 15]);")
    lines.append("}")
    return "\n".join(lines) + "\n"


def run(cmd, timeout=30):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stderr
    except Exception as e:  # noqa: BLE001
        return "ERR", repr(e)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ccc", required=True)
    ap.add_argument("--gcc", default="/usr/bin/gcc")
    ap.add_argument("--seeds", default="0:100")
    ap.add_argument("--levels", default="O0,O2,Os")
    ap.add_argument("--out", default="/tmp/m32fz")
    args = ap.parse_args()

    lo, hi = (int(x) for x in args.seeds.split(":"))
    levels = args.levels.split(",")
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    results = {"pass": 0, "mismatch": 0, "lccc_compile_failure": 0, "skip": 0}
    failures = []
    for seed in range(lo, hi):
        src = out / f"s{seed}.c"
        src.write_text(gen_c(seed))
        drv = out / f"d{seed}.s"
        drv.write_text(DRIVER.replace("SEEDVAL", str(0x1234 + seed * 7919)))
        for level in levels:
            common = ["-m32", "-fno-PIE", "-fomit-frame-pointer", f"-{level}", "-w"]
            lo_obj = out / f"s{seed}_{level}_l.o"
            go_obj = out / f"s{seed}_{level}_g.o"
            rc, err = run([args.ccc, *common, "-c", str(src), "-o", str(lo_obj)])
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
