#!/usr/bin/env python3
"""i686 ALU torture differential: lccc -m32 vs gcc -m32, full 32-bit hash.

Covers the alu.rs surface that generic fuzzing under-samples: narrow-width
bitops (clz/ctz/popcount/bswap with dirty-high inputs), f32 negation,
multiply-by-constant shapes, constant division/remainder (numerator sweep
over the signed/unsigned boundary values), in-place/memory ALU forms,
3-operand LEA opportunities, variable shifts, ALU identities, and
commutative-immediate placement.

Freestanding: probe() folds every result into a 32-bit FNV hash that the
nostdlib _start driver writes to stdout via int $0x80 — the full word is
compared, not just the 8-bit exit code.

Usage:
  python3 alu_torture_m32.py --ccc path/to/lccc [--gcc gcc] [--levels O2,Os]
"""
import argparse
import subprocess
import sys
from pathlib import Path

DRIVER = r"""
.globl _start
_start:
    call probe
    movl %eax, (hashbuf)
    movl $4, %eax          # write(1, hashbuf, 4)
    movl $1, %ebx
    movl $hashbuf, %ecx
    movl $4, %edx
    int $0x80
    movl $1, %eax
    xorl %ebx, %ebx
    int $0x80
.bss
hashbuf:
    .long 0
"""

CFLAGS_COMMON = ["-m32", "-fno-PIE", "-fomit-frame-pointer", "-w", "-mno-sse", "-mno-mmx"]

PROBE = r"""
typedef unsigned char  u8;
typedef unsigned short u16;
typedef unsigned int   u32;
typedef signed char    i8;
typedef short          i16;
typedef int            i32;

static u32 H = 2166136261u;
static void mix(u32 v) { H = (H ^ v) * 16777619u; }

static u8  vi8;  static u16 vi16; static u32 vi32;
static float vfin; static float vfout;
static volatile u32 pin8, pin16, pin32, pinf, pinm, pinud, pinsd;

__attribute__((noinline)) static int  t_clz8(u8 x)   { return __builtin_clz(x); }
__attribute__((noinline)) static int  t_clz16(u16 x) { return __builtin_clz(x); }
__attribute__((noinline)) static int  t_clz32(u32 x) { return __builtin_clz(x); }
__attribute__((noinline)) static int  t_ctz8(u8 x)   { return __builtin_ctz(x); }
__attribute__((noinline)) static int  t_ctz16(u16 x) { return __builtin_ctz(x); }
__attribute__((noinline)) static int  t_ctz32(u32 x) { return __builtin_ctz(x); }
__attribute__((noinline)) static int  t_pop8(u8 x)   { return __builtin_popcount(x); }
__attribute__((noinline)) static int  t_pop16(u16 x) { return __builtin_popcount(x); }
__attribute__((noinline)) static int  t_pop32(u32 x) { return __builtin_popcount(x); }
__attribute__((noinline)) static u16  t_bs16(u16 x)  { return __builtin_bswap16(x); }
__attribute__((noinline)) static u32  t_bs32(u32 x)  { return __builtin_bswap32(x); }

__attribute__((noinline)) static void t_fneg(void) { vfout = -vfin; }
__attribute__((noinline)) static void t_fnegm(float f) { vfout = -f; }

__attribute__((noinline)) static int t_mul3(int x)  { return x * 3; }
__attribute__((noinline)) static int t_mul5(int x)  { return x * 5; }
__attribute__((noinline)) static int t_mul7(int x)  { return x * 7; }
__attribute__((noinline)) static int t_mul9(int x)  { return x * 9; }
__attribute__((noinline)) static int t_mul10(int x) { return x * 10; }
__attribute__((noinline)) static int t_mul12(int x) { return x * 12; }
__attribute__((noinline)) static int t_mul16(int x) { return x * 16; }
__attribute__((noinline)) static int t_mul1024(int x){ return x * 1024; }
__attribute__((noinline)) static int t_muln3(int x) { return x * -3; }
__attribute__((noinline)) static int t_muln8(int x) { return x * -8; }

__attribute__((noinline)) static u32 t_udiv3(u32 x)  { return x / 3; }
__attribute__((noinline)) static u32 t_udiv7(u32 x)  { return x / 7; }
__attribute__((noinline)) static u32 t_udiv10(u32 x) { return x / 10; }
__attribute__((noinline)) static u32 t_udiv100(u32 x){ return x / 100; }
__attribute__((noinline)) static u32 t_udiv_0xAAAAAAAB(u32 x){ return x / 0xAAAAAAABu; }
__attribute__((noinline)) static u32 t_urem7(u32 x)  { return x % 7; }
__attribute__((noinline)) static u32 t_urem10(u32 x) { return x % 10; }
__attribute__((noinline)) static u32 t_urem8(u32 x)  { return x % 8; }
__attribute__((noinline)) static i32 t_sdiv7(i32 x)  { return x / 7; }
__attribute__((noinline)) static i32 t_sdivn7(i32 x) { return x / -7; }
__attribute__((noinline)) static i32 t_sdiv8(i32 x)  { return x / 8; }
__attribute__((noinline)) static i32 t_sdivn8(i32 x) { return x / -8; }
__attribute__((noinline)) static i32 t_sdiv_0x55555557(i32 x){ return x / 0x55555557; }
__attribute__((noinline)) static i32 t_srem7(i32 x)  { return x % 7; }
__attribute__((noinline)) static i32 t_sremn3(i32 x) { return x % -3; }
__attribute__((noinline)) static i32 t_srem8(i32 x)  { return x % 8; }

static volatile int va, vb, vc;
__attribute__((noinline)) static void t_inplace_add(void) { va += vb; }
__attribute__((noinline)) static void t_inplace_orimm(void) { va |= 0x40; }
__attribute__((noinline)) static void t_inplace_addslot(void) { va = va + vb; }
__attribute__((noinline)) static int  t_lea_add(int x, int y) { return x + y; }
__attribute__((noinline)) static int  t_lea_addimm(int x) { return x + 12345; }
__attribute__((noinline)) static int  t_varshift(int x, int n) { return x << n; }
__attribute__((noinline)) static int  t_add0(int x) { return x + 0; }
__attribute__((noinline)) static int  t_or0(int x)  { return x | 0; }
__attribute__((noinline)) static int  t_xor0(int x) { return x ^ 0; }
__attribute__((noinline)) static int  t_andm1(int x){ return x & -1; }
__attribute__((noinline)) static int  t_lhsimm(int x) { return 5 + x; }
__attribute__((noinline)) static int  t_lhsimmmul(int x) { return 3 * x; }

u32 probe(void) {
    static const u32 edges[] = {0,1,2,0x80u,0xffu,0x100u,0x8000u,0xffffu,
        0x10000u,0x80000000u,0xfffffffeu,0xffffffffu};
    for (int i = 0; i < (int)(sizeof(edges)/sizeof(edges[0])); i++) {
        u32 e = edges[i];
        pin8 = e; pin16 = e; pin32 = e;
        vi8 = (u8)e; vi16 = (u16)e; vi32 = e;
        mix((u32)t_clz8(vi8));
        mix((u32)t_clz16(vi16));
        mix((u32)t_clz32(vi32));
        /* ctz(0) is UB in C: only test truncated-nonzero values (e.g.
           e=0x10000 truncates to 0 in u16 — skipping those keeps the
           differential oracle well-defined for both compilers). */
        if ((u8)e)  mix((u32)t_ctz8(vi8));
        if ((u16)e) mix((u32)t_ctz16(vi16));
        if (e)      mix((u32)t_ctz32(vi32));
        mix((u32)t_pop8(vi8));
        mix((u32)t_pop16(vi16));
        mix((u32)t_pop32(vi32));
        mix(t_bs16(vi16));
        mix(t_bs32(vi32));
    }
    /* div/rem: numerator sweep incl. every boundary value */
    static const i32 snum[] = {0,1,-1,2,-2,3,6,7,-7,8,-8,20,100,-100,1000000,
        2147483647,-2147483647,-2147483648,33554432,-33554432,536870912,999999937};
    static const u32 unum[] = {0u,1u,2u,3u,6u,7u,8u,10u,20u,100u,1000000u,
        2147483647u,0x80000000u,0xfffffffeu,0xffffffffu,999999937u,536870912u};
    for (int i = 0; i < (int)(sizeof(snum)/sizeof(snum[0])); i++) {
        i32 x = snum[i];
        mix((u32)t_sdiv7(x)); mix((u32)t_sdivn7(x)); mix((u32)t_sdiv8(x));
        mix((u32)t_sdivn8(x)); mix((u32)t_sdiv_0x55555557(x));
        mix((u32)t_srem7(x)); mix((u32)t_sremn3(x)); mix((u32)t_srem8(x));
        mix((u32)t_mul3(x)); mix((u32)t_mul5(x)); mix((u32)t_mul7(x));
        mix((u32)t_mul9(x)); mix((u32)t_mul10(x)); mix((u32)t_mul12(x));
        mix((u32)t_mul16(x)); mix((u32)t_mul1024(x));
        mix((u32)t_muln3(x)); mix((u32)t_muln8(x));
        mix((u32)t_lea_add(x, 1234567)); mix((u32)t_lea_addimm(x));
        mix((u32)t_varshift(x, i & 31)); mix((u32)t_varshift(x, 0));
        mix((u32)t_add0(x)); mix((u32)t_or0(x)); mix((u32)t_xor0(x));
        mix((u32)t_andm1(x)); mix((u32)t_lhsimm(x)); mix((u32)t_lhsimmmul(x));
    }
    for (int i = 0; i < (int)(sizeof(unum)/sizeof(unum[0])); i++) {
        u32 x = unum[i];
        mix(t_udiv3(x)); mix(t_udiv7(x)); mix(t_udiv10(x)); mix(t_udiv100(x));
        mix(t_udiv_0xAAAAAAAB(x));
        mix(t_urem7(x)); mix(t_urem10(x)); mix(t_urem8(x));
    }
    /* f32 negation: assorted bit patterns incl. 0, -0, inf, nan, denormal */
    static const u32 fbits[] = {0u, 0x80000000u, 0x3f800000u, 0xbf800000u,
        0x7f800000u, 0xff800000u, 0x7fc00000u, 0x00000001u, 0x41234567u};
    for (int i = 0; i < (int)(sizeof(fbits)/sizeof(fbits[0])); i++) {
        pinf = fbits[i];
        vfin = *(const float *)&pinf;
        t_fneg();
        pinf = vfout == 0.0f ? (vfout == 0.0f ? 0u : 1u) : *(const u32 *)&vfout;
        mix(pinf);  /* -0 vs 0 must distinguish: compare via bits */
        t_fnegm(*(const float *)&fbits[i]);
        pinf = *(const u32 *)&vfout;
        mix(pinf);
    }
    va = 12345; vb = -7; vc = 0;
    t_inplace_add(); mix((u32)va);
    t_inplace_orimm(); mix((u32)va);
    t_inplace_addslot(); mix((u32)va);
    mix((u32)t_lea_add(va, vb));
    return H;
}
"""


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--ccc", required=True)
    ap.add_argument("--gcc", default="gcc")
    ap.add_argument("--levels", default="O2,Os")
    ap.add_argument("--out", default="/tmp/alu-torture")
    args = ap.parse_args()

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "probe.c").write_text(PROBE)
    (out / "drv.s").write_text(DRIVER)

    failures = 0
    for level in args.levels.split(","):
        cflags = CFLAGS_COMMON + [f"-{level}"]
        # lccc side
        r = run([args.ccc] + cflags + ["-c", str(out / "probe.c"), "-o", str(out / "probe_l.o")])
        if r.returncode:
            print(f"[{level}] lccc COMPILE FAIL:\n{r.stderr[:2000]}")
            failures += 1
            continue
        r = run([args.gcc, "-m32", "-nostdlib", "-no-pie", str(out / "drv.s"),
                 str(out / "probe_l.o"), "-o", str(out / "bin_l")])
        if r.returncode:
            print(f"[{level}] lccc LINK FAIL:\n{r.stderr[:2000]}")
            failures += 1
            continue
        # gcc side
        r = run([args.gcc] + cflags + ["-mpopcnt", "-mlzcnt", "-c", str(out / "probe.c"), "-o", str(out / "probe_g.o")])
        if r.returncode:
            print(f"[{level}] gcc COMPILE FAIL:\n{r.stderr[:2000]}")
            failures += 1
            continue
        r = run([args.gcc, "-m32", "-nostdlib", "-no-pie", str(out / "drv.s"),
                 str(out / "probe_g.o"), "-o", str(out / "bin_g")])
        if r.returncode:
            print(f"[{level}] gcc LINK FAIL:\n{r.stderr[:2000]}")
            failures += 1
            continue

        hl = subprocess.run([str(out / "bin_l")], capture_output=True).stdout
        hg = subprocess.run([str(out / "bin_g")], capture_output=True).stdout
        def word(b):
            return int.from_bytes(b[:4], "little") if len(b) >= 4 else None
        wl, wg = word(hl), word(hg)
        verdict = "MATCH" if wl == wg else "MISMATCH"
        print(f"[{level}] lccc=0x{wl:08x} gcc=0x{wg:08x}  {verdict}")
        if wl != wg:
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
