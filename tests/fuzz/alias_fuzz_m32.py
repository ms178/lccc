#!/usr/bin/env python3
"""Adversarial alias differential fuzz for redundant-load elimination + GVN.

Generates programs that are DESIGNED to break load merging: repeated field
accesses, stores between loads (same object, different object, overlapping
offsets, conditional stores), same-base pointer arithmetic with offsets that
converge/diverge across iterations, calls clobbering memory, f32/f64 load
pairs around stores (GVN float CSE), and memmove-style in-object copies.

lccc -m32 vs gcc -m32, full 32-bit hash via the nostdlib int80 driver.

Usage: alias_fuzz_m32.py --ccc path --seeds 0:60 [--levels O2,Os]
"""
import argparse
import random
import subprocess
import sys
from pathlib import Path

DRIVER = r"""
.globl _start
_start:
    call probe
    movl %eax, (hashbuf)
    movl $4, %eax
    movl $1, %ebx
    movl $hashbuf, %ecx
    movl $4, %edx
    int $0x80
    movl $1, %eax
    xorl %ebx, %ebx
    int $0x80
.bss
hashbuf: .long 0
"""


def gen(seed: int) -> str:
    r = random.Random(seed)
    lines = [
        "typedef unsigned int u32; typedef int i32;",
        "static struct S { int a; int b; float fa; float fb; } arr[48];",
        "static struct S arr2[48];",
        "static float fs[32];",
        "static int iarr[64];",
        "static u32 H = 2166136261u;",
        "static void mix(u32 v) { H = (H ^ v) * 16777619u; }",
        "static int opaque_callee(int *p) { return *p; }  /* memory op */",
        "/* gcc's loop-idiom recognition turns the in-object shift loop into a",
        " * memmove() call; define it so both compilers link freestanding. */",
        "void *memmove(void *d, const void *s, unsigned n) {",
        "  char *dd = d; const char *ss = s;",
        "  if (dd < ss) { for (unsigned i = 0; i < n; i++) dd[i] = ss[i]; }",
        "  else { for (unsigned i = n; i-- > 0; ) dd[i] = ss[i]; }",
        "  return d;",
        "}",
    ]
    # choose adversarial scenario per seed
    scen = seed % 8
    if scen == 0:
        # repeated same-field loads + interleaved disjoint stores
        lines += [
            "u32 probe(void) {",
            "  for (int j = 2; j < 40; j++) {",
            "    arr[j].a = arr[j-1].a + arr[j-2].a;",
            "    arr[j].b = arr[j].a * 3 - arr[j-1].b;",
            "    arr2[j].a = arr[j].b + 1;  /* different object */",
            "  }",
            "  for (int j = 0; j < 40; j++) { mix((u32)arr[j].a); mix((u32)arr[j].b); mix((u32)arr2[j].a); }",
            "  return H; }",
        ]
    elif scen == 1:
        # store to the SAME field between loads (must NOT merge)
        lines += [
            "u32 probe(void) {",
            "  int t = 0;",
            "  for (int j = 0; j < 30; j++) {",
            "    t += arr[j].a;",
            "    arr[j].a = j * 5;   /* invalidates the load above */",
            "    t ^= arr[j].a;      /* sees the NEW value */",
            "  }",
            "  mix((u32)t);",
            "  return H; }",
        ]
    elif scen == 2:
        # overlapping in-object copy (memmove semantics!)
        lines += [
            "u32 probe(void) {",
            "  for (int i = 0; i < 63; i++) iarr[i] = i;",
            "  for (int i = 0; i + 1 < 64; i++) iarr[i] = iarr[i+1]; /* shift left */",
            "  for (int i = 0; i < 64; i++) mix((u32)iarr[i]);",
            "  return H; }",
        ]
    elif scen == 3:
        # FP load CSE around stores to the SAME and OTHER floats
        lines += [
            "u32 probe(void) {",
            "  for (int i = 0; i < 31; i++) fs[i] = (float)i * 1.5f;",
            "  float s = 0.0f;",
            "  for (int i = 1; i < 31; i++) {",
            "    s += fs[i] + fs[i];       /* CSE candidate */",
            "    fs[i-1] = fs[i] + 1.0f;   /* must invalidate earlier */",
            "  }",
            "  mix(*(u32 *)&s);",
            "  for (int i = 0; i < 32; i++) mix(*(u32 *)&fs[i]);",
            "  return H; }",
        ]
    elif scen == 4:
        # conditional store (control-flow between loads) in a loop
        lines += [
            "u32 probe(void) {",
            "  for (int j = 3; j < 44; j++) {",
            "    int v = arr[j].a + arr[j].b;",
            "    if (v & 1) arr[j/2].a = v;   /* may alias arr[j]! */",
            "    mix((u32)(arr[j].a - arr[j].b));",
            "  }",
            "  return H; }",
        ]
    elif scen == 5:
        # pointer params walking the same object from two bases
        lines += [
            "static int walk(int *p, int *q, int n) {",
            "  int t = 0;",
            "  for (int i = 2; i < n; i++) { t += p[i]; q[i] = p[i-1] + p[i-2]; }",
            "  return t;",
            "}",
            "u32 probe(void) {",
            "  for (int i = 0; i < 64; i++) iarr[i] = (i*7) & 31;",
            "  mix((u32)walk(iarr, iarr + 1, 60));  /* overlapping! */",
            "  for (int i = 0; i < 64; i++) mix((u32)iarr[i]);",
            "  return H; }",
        ]
    elif scen == 6:
        # call between loads (memory clobber) + same-field reload
        lines += [
            "u32 probe(void) {",
            "  int t = 0;",
            "  for (int j = 0; j < 24; j++) {",
            "    t += arr[j].a;",
            "    t += opaque_callee(&arr[j].b);",
            "    t += arr[j].a;   /* call may have changed it */",
            "  }",
            "  mix((u32)t);",
            "  return H; }",
        ]
    else:
        # marching-pointer stores vs invariant field loads (nbody shape)
        lines += [
            "u32 probe(void) {",
            "  for (int i = 0; i < 40; i++) { arr[i].a = i; arr[i].b = -i; arr[i].fa = (float)i; }",
            "  for (int i = 1; i < 39; i++) {",
            "    int ax = arr[i].a, bx = arr[i].b;",
            "    for (int j = i + 1; j < 40; j++) {",
            "      arr[j].b += ax; arr[j].a -= bx;   /* marches past arr[i] */",
            "    }",
            "  }",
            "  for (int i = 0; i < 40; i++) { mix((u32)arr[i].a); mix((u32)arr[i].b); }",
            "  return H; }",
        ]
    # seed-dependent initial state so nothing folds to constants
    init = "static void __attribute__((constructor)) not_used(void) {}"
    lines.append(init)
    lines.insert(1, "static volatile u32 V = %du;" % (r.randint(1, 4000000000)))
    lines.append("/* seed %d */" % seed)
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ccc", required=True)
    ap.add_argument("--gcc", default="gcc")
    ap.add_argument("--seeds", default="0:64")
    ap.add_argument("--levels", default="O2,Os")
    ap.add_argument("--out", default="/tmp/alias-fuzz")
    args = ap.parse_args()

    lo, hi = (int(x) for x in args.seeds.split(":"))
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "drv.s").write_text(DRIVER)

    fails = 0
    for level in args.levels.split(","):
        mismatches = []
        for seed in range(lo, hi, int(__import__("os").environ.get("SEED_STEP","1"))):
            src = gen(seed)
            (out / "t.c").write_text(src)
            common = [x for x in [__import__("os").environ.get("EXTRA_FLAGS",""), "-m32", "-fno-PIE", "-w", "-mno-sse", "-mno-mmx", f"-{level}"] if x]
            r1 = subprocess.run([args.ccc] + common + ["-c", str(out / "t.c"), "-o", str(out / "l.o")], capture_output=True)
            if r1.returncode:
                mismatches.append(f"seed{seed}: lccc compile fail: {r1.stderr.decode()[:200]}")
                continue
            r2 = subprocess.run([args.gcc] + common + ["-c", str(out / "t.c"), "-o", str(out / "g.o")], capture_output=True)
            if r2.returncode:
                mismatches.append(f"seed{seed}: gcc compile fail (test bug)")
                continue
            for o, b in (("l.o", "bl"), ("g.o", "bg")):
                subprocess.run([args.gcc, "-m32", "-nostdlib", "-no-pie", str(out / "drv.s"), str(out / o), "-o", str(out / b)], capture_output=True)
            hl = subprocess.run([str(out / "bl")], capture_output=True).stdout[:4]
            hg = subprocess.run([str(out / "bg")], capture_output=True).stdout[:4]
            if hl != hg:
                mismatches.append(f"seed{seed}: lccc={hl.hex()} gcc={hg.hex()} (scen {seed % 8})")
        print(f"[{level}] {hi-lo} programs, {len(mismatches)} mismatches")
        for m in mismatches[:10]:
            print("   ", m)
        fails += len(mismatches)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
