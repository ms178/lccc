#!/usr/bin/env python3
"""AArch64 differential fuzz: lccc-arm vs aarch64-linux-gnu-gcc under qemu-user.

Freestanding probe + nostdlib _start that writes the 32-bit result hash via
the write syscall; both binaries run under qemu-aarch64-static.

Usage: aarch64_fuzz.py --ccc path/to/lccc-arm [--seeds 0:40] [--levels O2,Os]
"""
import argparse
import os
import random
import subprocess
import sys
from pathlib import Path

DRIVER = r"""
.globl _start
_start:
    stp x29, x30, [sp, #-16]!
    mov x0, #1
    mov x1, #2
    bl probe
    ldr x1, =hashbuf
    str w0, [x1]
    mov x0, #1              // fd stdout
    ldr x1, =hashbuf
    mov x2, #4
    mov x8, #64             // write
    svc #0
    mov x0, #0
    mov x8, #93             // exit
    svc #0
.bss
hashbuf: .word 0
"""


def gen(seed: int) -> str:
    r = random.Random(seed)
    scen = seed % 5
    lines = [
        "typedef unsigned int u32; typedef int i32; typedef unsigned long long u64;",
        "static u32 H = 2166136261u;",
        "static void mix(u32 v) { H = (H ^ v) * 16777619u; }",
        "static u32 f2u(float f) { union { float f; u32 u; } c; c.f = f; return c.u; }",
        "static u32 d2lo(double d) { union { double d; u64 u; } c; c.d = d; return (u32)c.u; }",
        "static u32 d2hi(double d) { union { double d; u64 u; } c; c.d = d; return (u32)(c.u >> 32); }",
    ]
    if scen == 0:
        # FP-heavy loop (FP CSE + phi coalescing + FP register pressure)
        lines += [
            "static float fa[64]; static double da[64];",
            "u32 probe(i32 seed, i32 it) {",
            "  float fs = 0.0f; double ds = 0.0;",
            "  for (int i = 0; i < 48; i++) { fa[i] = (float)(i * seed); da[i] = (double)i * 1.5; }",
            "  for (int i = 1; i < 47; i++) {",
            "    fs += fa[i] * fa[i-1] + fa[i+1];",
            "    ds += da[i] / 3.0 - da[i-1];",
            "    fa[i] = fs; da[i] = ds;   /* store invalidates earlier loads */",
            "  }",
            "  mix(f2u(fs)); mix(d2lo(ds)); mix(d2hi(ds));",
            "  for (int i = 0; i < 64; i += 7) { mix(f2u(fa[i])); mix(d2hi(da[i])); }",
            "  return H;",
            "}",
        ]
    elif scen == 1:
        # select/conditional + i8 sign-extension (sxtb elimination target)
        lines += [
            "static signed char sc[64]; static int sel_acc;",
            "u32 probe(i32 seed, i32 it) {",
            "  for (int i = 0; i < 60; i++) sc[i] = (signed char)(i * seed);",
            "  for (int i = 1; i < 59; i++) {",
            "    int v = sc[i] < sc[i-1] ? sc[i] + 1 : sc[i-1] - 1;",
            "    sel_acc += (v & 0x80) ? -v : v;",
            "    sc[i+1] = (signed char)v;",
            "  }",
            "  mix((u32)sel_acc);",
            "  for (int i = 0; i < 64; i += 5) mix((u32)(signed int)sc[i]);",
            "  return H;",
            "}",
        ]
    elif scen == 2:
        # loop-promoted FP values (the d24..d31 pool) + mixed int work
        lines += [
            "static double m[16][16]; static int im[64];",
            "u32 probe(i32 seed, i32 it) {",
            "  double a = 1.0, b = 2.0, c = 3.0, d = 4.0, e = 5.0, f = 6.0, g = 7.0, h = 8.0;",
            "  for (int i = 0; i < 64; i++) im[i] = i ^ seed;",
            "  for (int i = 0; i < 16; i++)",
            "    for (int j = 0; j < 16; j++)",
            "      m[i][j] = (double)((im[i & 63] + j) & 31);",
            "  for (int k = 0; k < 40; k++) {",
            "    a += m[k & 15][k & 15] * 0.25; b -= a * 0.5; c += b * 0.125;",
            "    d += c * 0.0625; e += d * 2.0; f -= e; g += f; h = h * 1.01 + g * 0.001;",
            "  }",
            "  mix(f2u(a)); mix(d2hi(h));",
            "  return H;",
            "}",
        ]
    elif scen == 3:
        # alias/redundant-load scenarios ported to aarch64
        lines += [
            "static struct S { int a; int b; } arr[48];",
            "u32 probe(i32 seed, i32 it) {",
            "  for (int i = 0; i < 40; i++) { arr[i].a = i * seed; arr[i].b = -i; }",
            "  for (int i = 1; i < 39; i++) {",
            "    int ax = arr[i].a, bx = arr[i].b;",
            "    for (int j = i + 1; j < 40; j++) { arr[j].b += ax; arr[j].a -= bx; }",
            "  }",
            "  for (int i = 0; i < 40; i++) { mix((u32)arr[i].a); mix((u32)arr[i].b); }",
            "  return H;",
            "}",
        ]
    else:
        # triangular indices (quadratic_sr) + general int soup
        lines += [
            "static long long acc;",
            "u32 probe(i32 seed, i32 it) {",
            "  i32 n = 12 + (seed & 7);",
            "  for (i32 i = (seed & 15) - 8; i < n; i++) {",
            "    for (i32 j = 0; j < n; j++) {",
            "      i32 t = i + j;",
            "      i32 idx = t*(t+1)/2 + i + 1;",
            "      acc += idx;",
            "    }",
            "  }",
            "  mix((u32)acc); mix((u32)(acc >> 32));",
            "  return H;",
            "}",
        ]
    lines.insert(1, "static volatile u32 V = %du;" % r.randint(1, 3000000000))
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ccc", required=True)
    ap.add_argument("--gcc", default="aarch64-linux-gnu-gcc")
    ap.add_argument("--qemu", default="qemu-aarch64")
    ap.add_argument("--seeds", default="0:40")
    ap.add_argument("--levels", default="O2,Os")
    ap.add_argument("--out", default="/tmp/aa64-fuzz")
    args = ap.parse_args()

    lo, hi = (int(x) for x in args.seeds.split(":"))
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "drv.s").write_text(DRIVER)
    qemu = args.qemu

    fails = 0
    for level in args.levels.split(","):
        mismatches = []
        for seed in range(lo, hi):
            src = gen(seed)
            (out / "t.c").write_text(src)
            common = ["-fno-PIE", "-w", "-fno-strict-aliasing", f"-{level}"]
            r1 = subprocess.run([args.ccc] + common + ["-c", str(out / "t.c"), "-o", str(out / "l.o")], capture_output=True)
            if r1.returncode:
                mismatches.append(f"seed{seed}: lccc compile fail: {r1.stderr.decode()[:160]}")
                continue
            r2 = subprocess.run([args.gcc] + common + ["-c", str(out / "t.c"), "-o", str(out / "g.o")], capture_output=True)
            if r2.returncode:
                mismatches.append(f"seed{seed}: gcc fail (test bug): {r2.stderr.decode()[:120]}")
                continue
            for o, b in (("l.o", "bl"), ("g.o", "bg")):
                subprocess.run([args.gcc, "-nostdlib", "-no-pie", str(out / "drv.s"), str(out / o), "-o", str(out / b)], capture_output=True)
            hl = subprocess.run([qemu, str(out / "bl")], capture_output=True).stdout[:4]
            hg = subprocess.run([qemu, str(out / "bg")], capture_output=True).stdout[:4]
            if hl != hg:
                mismatches.append(f"seed{seed}: lccc={hl.hex()} gcc={hg.hex()} (scen {seed % 5})")
        print(f"[{level}] {hi-lo} programs, {len(mismatches)} mismatches")
        for m in mismatches[:8]:
            print("   ", m)
        fails += len(mismatches)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
