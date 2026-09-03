#!/usr/bin/env python3
"""Red-team differential tester for the i686 ALU lowering (``alu.rs``).

The i686 backend folds constant division/remainder (magic numbers, signed
power-of-two bias), strength-reduces constant multiplication (LEA/shift
chains), fuses same-block div/rem pairs and emits width-aware clz/ctz/
popcount/bswap.  Every one of those paths is a hand-written instruction
sequence whose correctness depends on subtle arithmetic (Hacker's Delight
magic-number selection, the "add" indicator, arithmetic-vs-logical sign
masks, 5-bit shift masking).  A single wrong constant miscompiles silently.

This tool generates a *deterministic* corpus of tiny ``noinline`` functions
that drives every lowering with edge-case and pseudo-random inputs, compiles
it with ``lccc-i686`` at several optimisation levels (and optional ISA flags),
runs the 32-bit binaries natively and compares per-function FNV-1a hashes with
``gcc -m32 -O0`` (ground truth: GCC at -O0 uses plain ``idiv``/``imul``, so a
disagreement is attributable to the LCCC strength-reduction path named in the
mismatching line).  The reference is cross-checked against ``gcc -O2`` so a
UB-containing corpus can never masquerade as an LCCC bug.

Corpus layout (each function prints ``<name> <hash>``):

* ``udiv_D/urem_D/sdiv_D/srem_D``  constant division & remainder for every
  divisor class: 2..64 exhaustively, primes, 2^k±1, magic "add"-indicator
  divisors (7, 14, 19, 21, 27, 31, 37, 39, 41, 47, ...), 10^k, 2^31-1,
  2^31, 2^31+1, 2^32-1, negative divisors, INT_MIN.
* ``udr_D/sdr_D/urd_D/srd_D``  same-block div+rem pairs (div first / rem
  first) — the pair-fusion path with its store-order hazard; ``uqr_D`` keeps
  the quotient live across a store between the two.
* ``mul_C/mulk_C/mull_C``  multiply by C in the accumulator path, in the
  direct-to-dest path with the source kept live (forces src != dest), and
  with a loop-carried source (LEA chains that must not clobber src).
* ``shl_K/shr_K/sar_K``  every immediate shift 0..31 plus variable shifts
  and rotate idioms.
* ``bit_*``  clz/ctz/popcount/bswap16/bswap32/parity/ffs/clrsb and bit-test
  idioms with sign- and zero-extended narrow inputs.
* ``rnd_N``  seeded random expression trees mixing all of the above with
  UB-free operand shaping (unsigned wraparound, masked shift counts,
  non-zero positive variable divisors).

Usage::

    scripts/i686_alu_redteam.py                       # default: -O0 -O1 -O2 -O3 -Os
    scripts/i686_alu_redteam.py --levels -O2 -Os --extra -mpopcnt -mlzcnt
    scripts/i686_alu_redteam.py --keep out/           # keep generated sources
    scripts/i686_alu_redteam.py --stats               # static insn census vs gcc -O2

Exit status is non-zero on any mismatch, compile failure, crash or timeout.
"""
from __future__ import annotations

import argparse
import random
import re
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_LCCC = REPO / "target" / "fastbuild" / "lccc-i686"

# ── Divisor classes ──────────────────────────────────────────────────────────
_PRIMES = [67, 71, 73, 97, 101, 127, 131, 251, 257, 641, 1009, 4099, 65537,
           6700417, 2147483647]
_ADD_INDICATOR = [7, 14, 19, 21, 27, 28, 31, 35, 37, 39, 41, 47, 49, 53, 55,
                  57, 61, 62, 63, 91, 93, 95, 99, 105, 107, 109, 111, 113]
_POW2_NEIGHBOURS = [2 ** k + s for k in range(2, 32) for s in (-1, 1)]
_ROUND = [10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000,
          1000000000, 60, 3600, 86400, 24, 12, 365, 1024, 4096, 65536,
          16777216, 0x7FFFFFFF, 0x80000000, 0x80000001, 0xFFFFFFFE,
          0xFFFFFFFF, 0xAAAAAAAB, 0xCCCCCCCD]


def _uniq(xs):
    seen, out = set(), []
    for x in xs:
        if x not in seen:
            seen.add(x)
            out.append(x)
    return out


UDIVISORS = _uniq([d for d in list(range(2, 65)) + _PRIMES + _ADD_INDICATOR
                   + _POW2_NEIGHBOURS + _ROUND if 2 <= d <= 0xFFFFFFFF])
SDIVISORS = _uniq([d for d in UDIVISORS if d <= 0x7FFFFFFF]
                  + [-d for d in UDIVISORS if d <= 0x7FFFFFFF]
                  + [-2147483647 - 1])

MUL_CONSTS = _uniq(list(range(-70, 141)) + [
    0x100, 0x101, 0xFF, 0x1FF, 0x200, 0x3FF, 0x400, 0x7FF, 0x1000, 0xFFFF,
    0x10000, 0x10001, 0x01010101, 0x55555555, 0x7FFFFFFF, -0x7FFFFFFF - 1,
    0x40000000, 0x20000000, 0x00FF00FF, 1000, 1024, 1023, 1025, 3600, 65535,
    0x9E3779B9 - (1 << 32), 0x811C9DC5 - (1 << 32), 16777619,
    2654435761 - (1 << 32), 144, 160, 192, 288, 320, 576, 640,
])

EDGE_INPUTS = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 15, 16, 17, 31, 32, 33, 63, 64, 65,
    100, 127, 128, 129, 255, 256, 257, 1000, 1023, 1024, 1025, 65535, 65536,
    65537, 0x7FFFFFFF, 0x80000000, 0x80000001, 0xFFFFFFFF, 0xFFFFFFFE,
    0xFFFFFFFD, 0xFFFF0000, 0x0000FFFF, 0x55555555, 0xAAAAAAAA, 0x12345678,
    0xDEADBEEF, 0xCAFEBABE, 0x7FFFFFFE, 0x40000000, 0x3FFFFFFF, 0xC0000000,
    0xBFFFFFFF, 0x00010000, 0x00008000, 0x00007FFF, 0x80007FFF, 0xFFFF8000,
    0xFFFF7FFF, 0x0000FF00, 0x00FF0000, 0xFF000000, 0x01010101, 0x10101010,
]


def _rng_inputs(n: int, seed: int) -> list[int]:
    r = random.Random(seed)
    out = []
    for _ in range(n):
        k = r.random()
        if k < 0.3:
            out.append(r.getrandbits(32))
        elif k < 0.5:
            out.append(r.getrandbits(r.randint(1, 31)))
        elif k < 0.7:
            out.append((-r.getrandbits(r.randint(1, 31))) & 0xFFFFFFFF)
        else:
            base = r.choice(EDGE_INPUTS)
            out.append((base + r.randint(-3, 3)) & 0xFFFFFFFF)
    return out


# ── C emission helpers ───────────────────────────────────────────────────────
def _c_int(v: int) -> str:
    """Portable C spelling of a 32-bit signed constant (INT_MIN safe)."""
    if v == -2147483648:
        return "(-2147483647 - 1)"
    return str(v)


def _c_uint(v: int) -> str:
    return f"{v}u"


def _tag(v: int) -> str:
    return f"m{-v}" if v < 0 else str(v)


HEADER = r"""
/* AUTO-GENERATED by scripts/i686_alu_redteam.py — do not edit. */
#include <stdio.h>
#include <stdint.h>
typedef uint32_t u32; typedef int32_t s32;
#define NI __attribute__((noinline))
static u32 fnv(u32 h, u32 v) { h ^= v; h *= 16777619u; h ^= v >> 16; h *= 16777619u; return h; }
static const u32 INPUTS[] = { %(inputs)s };
enum { NIN = sizeof INPUTS / sizeof INPUTS[0] };
static volatile u32 sink;
"""

FOOTER = r"""
int main(void) {
%(calls)s
    return 0;
}
"""


class Corpus:
    def __init__(self, seed: int, rnd_count: int):
        self.fns: list[str] = []
        self.calls: list[str] = []
        self.seed = seed
        self.rnd_count = rnd_count

    # -- unary over INPUTS -------------------------------------------------
    def unary(self, name: str, ret: str, arg: str, body: str):
        self.fns.append(f"NI {ret} {name}({arg} x) {{ {body} }}")
        self.calls.append(
            f"    {{ u32 h = 2166136261u; for (unsigned i = 0; i < NIN; i++) "
            f"h = fnv(h, (u32){name}(({arg})INPUTS[i])); "
            f"printf(\"{name} %08x\\n\", h); }}")

    # -- binary over INPUTS x INPUTS (strided to bound runtime) -------------
    def binary(self, name: str, ret: str, a: str, b: str, body: str, stride: int = 3):
        self.fns.append(f"NI {ret} {name}({a} x, {b} y) {{ {body} }}")
        self.calls.append(
            f"    {{ u32 h = 2166136261u; for (unsigned i = 0; i < NIN; i++) "
            f"for (unsigned j = i % {stride}; j < NIN; j += {stride}) "
            f"h = fnv(h, (u32){name}(({a})INPUTS[i], ({b})INPUTS[j])); "
            f"printf(\"{name} %08x\\n\", h); }}")

    # -- div/rem pair: returns q, stores r via pointer ----------------------
    def pair(self, name: str, ty: str, q_expr: str, r_expr: str, rem_first: bool):
        if rem_first:
            body = f"{ty} r = {r_expr}; {ty} q = {q_expr}; *rp = r; return q;"
        else:
            body = f"{ty} q = {q_expr}; {ty} r = {r_expr}; *rp = r; return q;"
        self.fns.append(f"NI {ty} {name}({ty} x, {ty} *rp) {{ {body} }}")
        self.calls.append(
            f"    {{ u32 h = 2166136261u; {ty} r; for (unsigned i = 0; i < NIN; i++) "
            f"{{ h = fnv(h, (u32){name}(({ty})INPUTS[i], &r)); h = fnv(h, (u32)r); }} "
            f"printf(\"{name} %08x\\n\", h); }}")

    def build(self) -> None:
        # ── constant division / remainder ──
        for d in UDIVISORS:
            c = _c_uint(d)
            self.unary(f"udiv_{d}", "u32", "u32", f"return x / {c};")
            self.unary(f"urem_{d}", "u32", "u32", f"return x % {c};")
            self.pair(f"udr_{d}", "u32", f"x / {c}", f"x % {c}", False)
            self.pair(f"urd_{d}", "u32", f"x / {c}", f"x % {c}", True)
            # Pair with a use of q between the two (the fold r = x - q*d must
            # survive an intervening consumer of q).
            self.unary(f"uqr_{d}", "u32", "u32",
                       f"u32 q = x / {c}; sink = q; u32 r = x % {c}; return q * 3u + r;")
        for d in SDIVISORS:
            c = _c_int(d)
            t = _tag(d)
            # INT_MIN / -1 is the only UB shape; -1 is not in SDIVISORS.
            self.unary(f"sdiv_{t}", "s32", "s32", f"return x / {c};")
            self.unary(f"srem_{t}", "s32", "s32", f"return x % {c};")
            self.pair(f"sdr_{t}", "s32", f"x / {c}", f"x % {c}", False)
            self.pair(f"srd_{t}", "s32", f"x / {c}", f"x % {c}", True)
        # Narrow dividends (promoted): the lowering sees an I32 holding a
        # sign/zero-extended value — magic sequences must not assume range.
        for d in (3, 7, 10, 100, 255, 256, -3, -7, -128, 127):
            c = _c_int(d)
            t = _tag(d)
            self.unary(f"sdiv8_{t}", "s32", "signed char", f"return x / {c};")
            self.unary(f"srem16_{t}", "s32", "short", f"return x % {c};")
            self.unary(f"udiv16_{t}", "u32", "unsigned short", f"return x / (u32){c};")
        # ── variable division (the general idiv path, direct-divisor forms) ──
        self.binary("vudiv", "u32", "u32", "u32", "return x / (y | 1u);")
        self.binary("vurem", "u32", "u32", "u32", "return x % (y | 1u);")
        self.binary("vsdiv", "s32", "s32", "s32", "return x / (s32)((y & 0x7fffffff) | 1);")
        self.binary("vsrem", "s32", "s32", "s32", "return x % (s32)((y & 0x7fffffff) | 1);")
        self.binary("vsdivneg", "s32", "s32", "s32", "return x / -(s32)((y & 0x3fffffff) | 2);")
        self.binary("vsremneg", "s32", "s32", "s32", "return x % -(s32)((y & 0x3fffffff) | 2);")
        self.binary("vdivrem", "u32", "u32", "u32", "u32 d = y | 1u; return x / d + 7u * (x % d);")
        self.binary("vsdivrem", "s32", "s32", "s32", "s32 d = (s32)((y & 0x7fffffff) | 1); return x / d ^ (x % d);")
        # ── constant multiplication ──
        for m in MUL_CONSTS:
            c = _c_int(m)
            t = _tag(m)
            self.unary(f"mul_{t}", "u32", "u32", f"return x * (u32){c};")
            # direct-to-dest with src kept live (x reused after the multiply)
            self.binary(f"mulk_{t}", "u32", "u32", "u32", f"u32 t = x * (u32){c}; return t ^ (x + y);", 5)
            # dest homed differently from src across a loop (LEA chain must
            # read the *original* x each iteration)
            self.unary(f"mull_{t}", "u32", "u32",
                       f"u32 acc = 0; for (u32 i = 0; i < 5; i++) {{ acc += (x + i) * (u32){c}; acc ^= x; }} return acc;")
        # ── shifts ──
        for k in range(32):
            self.unary(f"shl_{k}", "u32", "u32", f"return x << {k};")
            self.unary(f"shr_{k}", "u32", "u32", f"return x >> {k};")
            self.unary(f"sar_{k}", "s32", "s32", f"return x >> {k};")
            self.binary(f"shlk_{k}", "u32", "u32", "u32", f"u32 t = x << {k}; return t ^ (x + y);", 7)
        self.binary("vshl", "u32", "u32", "u32", "return x << (y & 31);")
        self.binary("vshr", "u32", "u32", "u32", "return x >> (y & 31);")
        self.binary("vsar", "s32", "s32", "u32", "return x >> (y & 31);")
        self.binary("vshl_swap", "u32", "u32", "u32", "return y << (x & 31);")
        self.binary("vshl3", "u32", "u32", "u32",
                    "u32 a = x << (y & 31); u32 b = y << (x & 31); u32 c = (x ^ y) >> ((x + y) & 31); return a + b * 3u + c;")
        self.binary("rotl", "u32", "u32", "u32", "u32 n = y & 31; return (x << n) | (x >> ((32u - n) & 31));")
        self.binary("rotr", "u32", "u32", "u32", "u32 n = y & 31; return (x >> n) | (x << ((32u - n) & 31));")
        # ── bit builtins (zero inputs are guarded where C leaves them undefined) ──
        self.unary("bit_clz", "u32", "u32", "return x ? (u32)__builtin_clz(x) : 32u;")
        self.unary("bit_ctz", "u32", "u32", "return x ? (u32)__builtin_ctz(x) : 32u;")
        self.unary("bit_popc", "u32", "u32", "return (u32)__builtin_popcount(x);")
        self.unary("bit_popc8", "u32", "u32", "return (u32)__builtin_popcount((unsigned char)x) + (u32)__builtin_popcount((signed char)x);")
        self.unary("bit_popc16", "u32", "u32", "return (u32)__builtin_popcount((unsigned short)x) + (u32)__builtin_popcount((short)x);")
        self.unary("bit_clz8", "u32", "u32", "unsigned char c = (unsigned char)x; return c ? (u32)__builtin_clz(c) : 99u;")
        self.unary("bit_clz16s", "u32", "u32", "short c = (short)x; return c ? (u32)__builtin_clz((u32)c) : 99u;")
        self.unary("bit_ctz16", "u32", "u32", "unsigned short c = (unsigned short)x; return c ? (u32)__builtin_ctz(c) : 99u;")
        self.unary("bit_bswap32", "u32", "u32", "return __builtin_bswap32(x);")
        self.unary("bit_bswap16", "u32", "u32", "return (u32)__builtin_bswap16((unsigned short)x);")
        self.unary("bit_bswap16s", "s32", "s32", "return (short)__builtin_bswap16((unsigned short)x);")
        self.unary("bit_parity", "u32", "u32", "return (u32)__builtin_parity(x);")
        self.unary("bit_ffs", "u32", "u32", "return (u32)__builtin_ffs((int)x);")
        self.unary("bit_clrsb", "u32", "s32", "return (u32)__builtin_clrsb(x);")
        self.binary("bit_test", "u32", "u32", "u32", "return (x >> (y & 31)) & 1u;")
        self.unary("bit_test_imm", "u32", "u32", "return ((x >> 5) & 1u) + ((x >> 31) & 1u) * 2u + ((x >> 0) & 1u) * 4u;")
        self.binary("bit_cmpsub", "u32", "u32", "u32", "return (x - y) ^ (y - x) ^ (x & ~y) ^ (~x | y);")
        self.unary("neg_not", "u32", "u32", "return (u32)(-(s32)(x ^ 0x5a5a5a5au)) + ~x;")
        self.unary("abs32", "u32", "s32", "return x < 0 ? (u32)-(x + 1) + 1u : (u32)x;")
        # ── random expression trees ──
        self._random_trees()

    def _random_trees(self) -> None:
        r = random.Random(self.seed)
        udivs = [d for d in UDIVISORS if d < 0x80000000]
        sdivs = [d for d in SDIVISORS if d != -2147483647 - 1]

        def expr(depth: int) -> str:
            if depth == 0:
                return r.choice(["x", "y", "z", "x", "y",
                                 f"{_c_uint(r.getrandbits(32))}",
                                 f"{r.randint(0, 200)}u"])
            op = r.random()
            a = expr(depth - 1)
            b = expr(depth - 1)
            if op < 0.10:
                return f"({a} + {b})"
            if op < 0.18:
                return f"({a} - {b})"
            if op < 0.26:
                return f"({a} * {_c_uint(r.choice(MUL_CONSTS) & 0xFFFFFFFF)})"
            if op < 0.32:
                return f"({a} * {b})"
            if op < 0.38:
                return f"({a} & {b})"
            if op < 0.43:
                return f"({a} | {b})"
            if op < 0.48:
                return f"({a} ^ {b})"
            if op < 0.54:
                return f"({a} << {r.randint(0, 31)})"
            if op < 0.60:
                return f"({a} >> {r.randint(0, 31)})"
            if op < 0.64:
                return f"({a} << ({b} & 31u))"
            if op < 0.68:
                return f"({a} >> ({b} & 31u))"
            if op < 0.74:
                return f"({a} / {_c_uint(r.choice(udivs))})"
            if op < 0.80:
                return f"({a} % {_c_uint(r.choice(udivs))})"
            if op < 0.85:
                d = r.choice(sdivs)
                return f"((u32)((s32){a} / {_c_int(d)}))"
            if op < 0.90:
                d = r.choice(sdivs)
                return f"((u32)((s32){a} % {_c_int(d)}))"
            if op < 0.93:
                return f"({a} / ({b} | 1u))"
            if op < 0.96:
                return f"({a} % ({b} | 1u))"
            if op < 0.98:
                return f"((u32)((s32){a} / (s32)(({b} & 0x7fffffffu) | 1u)))"
            return f"((u32)__builtin_popcount({a}) + (u32)__builtin_clz({b} | 1u))"

        for n in range(self.rnd_count):
            depth = r.randint(2, 4)
            # Several statements so the register allocator sees pressure and
            # values compete for the %eax/%ecx/%edx homes the lowerings clobber.
            stmts = [f"u32 t{i} = {expr(depth)};" for i in range(r.randint(1, 4))]
            uses = " ^ ".join(f"(t{i} * {2 * i + 1}u)" for i in range(len(stmts)))
            body = " ".join(stmts) + f" return {uses};"
            self.fns.append(f"NI u32 rnd_{n}(u32 x, u32 y, u32 z) {{ {body} }}")
            self.calls.append(
                f"    {{ u32 h = 2166136261u; for (unsigned i = 0; i < NIN; i += 2) "
                f"for (unsigned j = 1; j < NIN; j += 5) "
                f"h = fnv(h, rnd_{n}(INPUTS[i], INPUTS[j], INPUTS[(i * 7 + j) % NIN])); "
                f"printf(\"rnd_{n} %08x\\n\", h); }}")

    def shards(self, per_shard: int) -> list[str]:
        """Split the corpus into independent translation units.

        Each shard carries its own ``main``: (a) a crash localises to a small
        set of functions, (b) shards compile in parallel, and (c) the
        compiler's per-function cost stays bounded (a single 2 000-call
        ``main`` is superlinear in every backend, LCCC included).
        """
        inputs = EDGE_INPUTS + _rng_inputs(40, self.seed)
        header = HEADER % {"inputs": ", ".join(f"0x{v:08x}u" for v in inputs)}
        out = []
        for i in range(0, len(self.fns), per_shard):
            out.append(header + "\n".join(self.fns[i:i + per_shard]) + "\n"
                       + FOOTER % {"calls": "\n".join(self.calls[i:i + per_shard])})
        return out


# ── Driver ───────────────────────────────────────────────────────────────────
def run(cmd, timeout=900, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, **kw)


def parse_hashes(out: str) -> dict[str, str]:
    res = {}
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 2:
            res[parts[0]] = parts[1]
    return res


def insn_census(asm: str) -> dict[str, Counter]:
    """Per-function instruction census of AT&T assembly."""
    fn, out = None, {}
    for line in asm.splitlines():
        m = re.match(r"^([A-Za-z_][\w.]*):", line)
        if m:
            fn = m.group(1)
            out[fn] = Counter()
            continue
        s = line.strip()
        if not fn or not s or s.startswith(".") or s.startswith("#"):
            continue
        out[fn]["insns"] += 1
        mnem = s.split()[0]
        out[fn][mnem] += 1
        if mnem in ("pushl", "popl"):
            out[fn]["stack_rt"] += 1
    return out


def _compile_and_run(args, cfile: Path, lvl: str, exe: Path, asm: Path):
    """Compile one shard at one level, run it; returns (status, hashes, log)."""
    cmd = [args.lccc, lvl, *args.extra, "-w", "-o", str(exe), str(cfile)]
    try:
        p = run(cmd)
    except subprocess.TimeoutExpired:
        return "COMPILE TIMEOUT", {}, ""
    if p.returncode:
        return f"COMPILE FAILED rc={p.returncode}", {}, p.stderr[-1500:]
    if args.stats:
        run([args.lccc, lvl, *args.extra, "-w", "-S", "-o", str(asm), str(cfile)])
    try:
        r = run([str(exe)], timeout=300)
    except subprocess.TimeoutExpired:
        return "RUN TIMEOUT", {}, ""
    if r.returncode != 0:
        return f"RUN CRASHED rc={r.returncode}", parse_hashes(r.stdout), r.stderr[-500:]
    return "OK", parse_hashes(r.stdout), ""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default=str(DEFAULT_LCCC))
    ap.add_argument("--levels", nargs="+", default=["-O0", "-O1", "-O2", "-O3", "-Os"])
    ap.add_argument("--extra", nargs="*", default=[], help="extra lccc flags (e.g. -mpopcnt -mlzcnt)")
    ap.add_argument("--seed", type=int, default=20260903)
    ap.add_argument("--rnd", type=int, default=240, help="number of random expression functions")
    ap.add_argument("--shard", type=int, default=96, help="functions per translation unit")
    ap.add_argument("-j", "--jobs", type=int, default=2)
    ap.add_argument("--keep", help="directory to keep sources/binaries/asm")
    ap.add_argument("--stats", action="store_true", help="print static census deltas vs gcc -m32 -O2")
    ap.add_argument("--gcc", default="gcc")
    args = ap.parse_args()

    corpus = Corpus(args.seed, args.rnd)
    corpus.build()
    shards = corpus.shards(args.shard)
    print(f"corpus: {len(corpus.fns)} functions in {len(shards)} shards, seed {args.seed}")

    work = Path(args.keep) if args.keep else Path(tempfile.mkdtemp(prefix="alu-redteam-"))
    work.mkdir(parents=True, exist_ok=True)
    cfiles = []
    for i, src in enumerate(shards):
        cf = work / f"shard{i:02d}.c"
        cf.write_text(src)
        cfiles.append(cf)

    # Ground truth: gcc -m32 -O0 (no strength reduction), cross-checked with -O2.
    ref: dict[str, str] = {}
    for cf in cfiles:
        b0, b2 = work / (cf.stem + "_gccO0"), work / (cf.stem + "_gccO2")
        p = run([args.gcc, "-m32", "-O0", "-w", "-o", str(b0), str(cf)])
        if p.returncode:
            print(f"gcc -m32 -O0 failed on {cf.name}:\n" + p.stderr)
            return 2
        h0 = parse_hashes(run([str(b0)]).stdout)
        p = run([args.gcc, "-m32", "-O2", "-w", "-o", str(b2), str(cf)])
        if p.returncode == 0:
            h2 = parse_hashes(run([str(b2)]).stdout)
            if h2 != h0:
                bad = [k for k in h0 if h0.get(k) != h2.get(k)]
                print(f"WARNING: gcc -O0 vs -O2 disagree on {bad[:5]} in {cf.name} — corpus has UB? aborting")
                return 2
        ref.update(h0)
    print(f"reference: {len(ref)} hashes from gcc -m32 -O0 (cross-checked with -O2)")

    import concurrent.futures as cf_
    failures = 0
    for lvl in args.levels:
        tag = lvl.lstrip("-") + ("_" + "_".join(f.lstrip("-") for f in args.extra) if args.extra else "")
        got: dict[str, str] = {}
        problems = []
        with cf_.ThreadPoolExecutor(max_workers=args.jobs) as ex:
            futs = {ex.submit(_compile_and_run, args, cf, lvl,
                              work / f"{cf.stem}_lccc_{tag}", work / f"{cf.stem}_lccc_{tag}.s"): cf
                    for cf in cfiles}
            for fut in cf_.as_completed(futs):
                status, hashes, log = fut.result()
                got.update(hashes)
                if status != "OK":
                    problems.append(f"{futs[fut].name}: {status} {log}")
        bad = [k for k in ref if got.get(k) != ref[k]]
        missing = [k for k in ref if k not in got]
        if problems or bad:
            failures += 1
            for pr in problems:
                print(f"[{lvl}] {pr}")
            if bad:
                print(f"[{lvl}] MISMATCH in {len(bad)} function(s): " + ", ".join(bad[:30]) + (" ..." if len(bad) > 30 else ""))
            if missing:
                print(f"[{lvl}]   ({len(missing)} functions produced no output)")
        else:
            print(f"[{lvl}] OK ({len(got)} functions match)")
        if args.stats:
            lc: dict = {}
            gc: dict = {}
            for cf in cfiles:
                asm = work / f"{cf.stem}_lccc_{tag}.s"
                gasm = work / f"{cf.stem}_gccO2.s"
                if asm.exists():
                    lc.update(insn_census(asm.read_text()))
                if not gasm.exists():
                    run([args.gcc, "-m32", "-O2", "-w", "-fno-asynchronous-unwind-tables", "-S", "-o", str(gasm), str(cf)])
                if gasm.exists():
                    gc.update(insn_census(gasm.read_text()))
            groups: Counter = Counter()
            for fn, c in lc.items():
                g = fn.split("_")[0]
                groups[(g, "lccc")] += c["insns"]
                groups[(g, "lccc_stack_rt")] += c["stack_rt"]
                if fn in gc:
                    groups[(g, "gcc")] += gc[fn]["insns"]
            print(f"[{lvl}] static census (total insns lccc vs gcc -O2; lccc push/pop):")
            for g in sorted({k[0] for k in groups}):
                print(f"    {g:8s} lccc={groups[(g, 'lccc')]:6d}  gcc={groups[(g, 'gcc')]:6d}  "
                      f"ratio={groups[(g, 'lccc')] / max(1, groups[(g, 'gcc')]):.2f}  push/pop={groups[(g, 'lccc_stack_rt')]}")
    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    else:
        print(f"artifacts kept in {work}")
    print("RESULT:", "FAIL" if failures else "PASS")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
