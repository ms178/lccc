#!/usr/bin/env python3
"""Differential stress test + missed-vectorization census for LCCC's vectorizer.

The research VM has no PMU, so evidence for `src/passes/vectorize.rs`,
`slp_vectorizer.rs`, `iv_widen.rs` and the IV strength-reduction passes has to
come from *coverage of the input space*.  This tool enumerates the loop-shape
space those passes pattern-match on and checks every point with three arms:

    gcc -O0                                                    independent reference
    lccc <flags>                                               vectorizer ON
    lccc <flags> + CCC_DISABLE_PASSES=vectorize,slp,slp_late,iv_widen   OFF

ON-vs-OFF isolates the vectorizer from the rest of the pipeline; the GCC arm
catches shared frontend bugs.  Any divergence is a miscompile and is reported
with a self-contained reproducer (``--keep DIR``).

Enumerated dimensions
---------------------
  * kernel       : reductions (sum, dot, sad, min, max, xor/or/and, masked
                   count, conditional sum, argmin, early-exit search,
                   ordered FP sum, two accumulators), maps (copy, add, mul,
                   shifts, abs, clamp, select, masked store, reverse,
                   interleave/deinterleave, 3-point stencil, in-place,
                   gather/scatter/histogram), loop-carried recurrences
                   (prefix sum, a[i+1]=a[i]) and width conversions.
  * element type : i8 u8 i16 u16 i32 u32 i64 u64 float double
  * IV type/form : int / unsigned / long / size_t  x  {lt, start1, step2, down}
  * trip count   : 0..9, 15..17, 31..33, 63..65, 100, 127..129, 255..257, 500
  * placement    : 4 (a,b,c) misalignment triples, plus -- for every map --
                   overlapping-buffer calls (a = p+off, b = p) with offsets in
                   {-4,-2,-1,0,1,2,3,5,8,9,16} that force the dependence /
                   alias analysis to decide correctly.
  * data profile : `full` range (extremes injected) for ops that cannot
                   overflow, `small` range otherwise, exact-integer floats
                   for reassociation-safe FP, and adversarial non-exact FP
                   (`fp_adv`) for the ordered-sum kernels: any illegal FP
                   reassociation changes the bits.

Every program hashes *entire* buffers including guard pads after each call, so
an out-of-range vector store, a masked store that wrote a false lane, or a
remainder-loop bug all change the checksum.

Missed-optimisation census
--------------------------
``--census`` additionally compiles each kernel with ``gcc -O3`` (same -march)
and counts packed-SIMD instructions inside the ``kernel`` symbol for both
compilers.  Kernels where GCC vectorizes and LCCC does not are ranked -- this
is the data-driven worklist for the vectorizer.

Usage (repo root, after a fastbuild)::

    scripts/vectorize_stress.py --lccc target/fastbuild/lccc-x86
    scripts/vectorize_stress.py --lccc target/fastbuild/lccc-x86 --census \
        --lccc-flags "-O3 -march=x86-64-v3" --json results/vec-census.json
    scripts/vectorize_stress.py --lccc target/fastbuild/lccc-x86 --filter sum --keep /tmp/vs

Exit status is 0 only when every configuration agrees across all arms.
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import json
import os
import random
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Dict, List, Optional, Sequence, Tuple

# ── Type model ────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Elem:
    tag: str  # short id
    c: str  # C spelling
    bits: int
    signed: bool
    fp: bool = False

    @property
    def unsigned_c(self) -> str:
        """Unsigned twin of an integer element type (`$U`).

        Signed accumulating kernels (`a[i] += cond`) would overflow -- undefined
        C -- when the `full` data profile injects INT_MAX / INT_MIN, so they do
        their arithmetic in the unsigned twin, whose wraparound is defined."""
        return self.c if not self.signed or self.fp else self.c.replace("int", "uint", 1)

    @property
    def wide(self) -> str:
        """Accumulator type that cannot overflow for n <= 500 on small data."""
        if self.fp:
            return "double"
        return "long long" if self.signed else "unsigned long long"


ELEMS = [
    Elem("i8", "int8_t", 8, True),
    Elem("u8", "uint8_t", 8, False),
    Elem("i16", "int16_t", 16, True),
    Elem("u16", "uint16_t", 16, False),
    Elem("i32", "int32_t", 32, True),
    Elem("u32", "uint32_t", 32, False),
    Elem("i64", "int64_t", 64, True),
    Elem("u64", "uint64_t", 64, False),
    Elem("f32", "float", 32, True, True),
    Elem("f64", "double", 64, True, True),
]
BY_TAG = {e.tag: e for e in ELEMS}

IV_TYPES = ["int", "unsigned", "long", "size_t"]
FORMS = ["lt", "start1", "step2", "down"]

TRIPS = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65,
         100, 127, 128, 129, 255, 256, 257, 500]
PLACEMENTS = [(0, 0, 0), (1, 0, 0), (0, 3, 1), (3, 2, 5)]
ALIAS_OFFSETS = [-4, -2, -1, 0, 1, 2, 3, 5, 8, 9, 16]

PAD = 32  # guard elements on either side of every buffer
NBUF = 1200  # payload elements per buffer


# ── Kernel description ───────────────────────────────────────────────────────


@dataclass(frozen=True)
class Kernel:
    name: str
    kind: str  # "red" | "map" | "rec"
    body: str  # C body using $-placeholders (see expand)
    types: Callable[[Elem], bool] = lambda e: True
    profile: str = "small"  # "full" | "small" | "fp_adv"
    acc: str = "wide"  # accumulator spelling: "wide" | "T"
    loop: bool = True  # supports IV-type/form rotation
    alias: bool = True  # maps: also run overlapping-buffer placements


def _any(_e: Elem) -> bool:
    return True


def _int(e: Elem) -> bool:
    return not e.fp


def _uint(e: Elem) -> bool:
    return (not e.fp) and (not e.signed)


def _signed_or_fp(e: Elem) -> bool:
    return e.signed


def _fp(e: Elem) -> bool:
    return e.fp


# Placeholders: $T element type, $U its UNSIGNED twin (wrap-defined
# arithmetic for signed accumulating kernels), $A accumulator type, $LOOP loop head (index i),
# $K small mid constant, $MAX / $MIN limits of T.
KERNELS: List[Kernel] = [
    # ── reductions ─────────────────────────────────────────────────────────
    Kernel("sum", "red", "$A s = 0; $LOOP s += b[i]; return s;"),
    Kernel("sum_T", "red", "$T s = 0; $LOOP s += b[i]; return s;", _any, "small", "T"),
    Kernel("sumsq", "red", "$A s = 0; $LOOP s += ($A)b[i] * b[i]; return s;"),
    Kernel("dot", "red", "$A s = 0; $LOOP s += ($A)b[i] * c[i]; return s;"),
    Kernel("sad", "red",
           "$A s = 0; $LOOP s += b[i] > c[i] ? ($A)(b[i] - c[i]) : ($A)(c[i] - b[i]); return s;"),
    Kernel("min", "red", "$T m = $MAX; $LOOP if (b[i] < m) m = b[i]; return m;", _any, "full"),
    Kernel("max", "red", "$T m = $MIN; $LOOP m = b[i] > m ? b[i] : m; return m;", _any, "full"),
    Kernel("xor", "red", "$T x = 0; $LOOP x ^= b[i]; return x;", _int, "full"),
    Kernel("or", "red", "$T x = 0; $LOOP x |= b[i]; return x;", _int, "full"),
    Kernel("and", "red", "$T x = ($T)-1; $LOOP x &= b[i]; return x;", _int, "full"),
    Kernel("count_nz", "red", "int cnt = 0; $LOOP if (b[i]) cnt++; return cnt;", _any, "full"),
    Kernel("count_gt", "red", "int cnt = 0; $LOOP if (b[i] > ($T)$K) cnt++; return cnt;", _any, "full"),
    Kernel("count_eq", "red", "int cnt = 0; $LOOP cnt += (b[i] == c2[0]); return cnt;", _any, "full"),
    Kernel("condsum", "red", "$A s = 0; $LOOP if (b[i] > ($T)$K) s += b[i]; return s;"),
    Kernel("condsum_sel", "red", "$A s = 0; $LOOP s += b[i] > ($T)$K ? b[i] : 0; return s;"),
    Kernel("twoacc", "red", "$A s = 0, t = 0; $LOOP { s += b[i]; t += c[i]; } return s * 31 + t;"),
    Kernel("argmin", "red",
           "$T m = $MAX; long ix = -1; $LOOP if (b[i] < m) { m = b[i]; ix = i; } return ix * 7 + (long long)m;",
           _any, "full"),
    Kernel("find", "red", "$LOOP if (b[i] == c2[0]) return i; return -1;", _any, "full"),
    Kernel("fsum_ordered", "red", "$T s = 0; $LOOP s += b[i]; return s;", _fp, "fp_adv", "T"),
    Kernel("fdot_ordered", "red", "$T s = 0; $LOOP s += b[i] * c[i]; return s;", _fp, "fp_adv", "T"),
    # ── maps ───────────────────────────────────────────────────────────────
    Kernel("copy", "map", "$LOOP a[i] = b[i];", _any, "full"),
    Kernel("add", "map", "$LOOP a[i] = b[i] + c[i];"),
    Kernel("sub", "map", "$LOOP a[i] = b[i] - c[i];"),
    Kernel("mul", "map", "$LOOP a[i] = b[i] * c[i];"),
    Kernel("scale_add", "map", "$LOOP a[i] = b[i] * 3 + 7;"),
    Kernel("inplace", "map", "$LOOP a[i] += b[i] * 3;"),
    Kernel("and_map", "map", "$LOOP a[i] = b[i] & c[i];", _int, "full"),
    Kernel("or_map", "map", "$LOOP a[i] = b[i] | c[i];", _int, "full"),
    Kernel("xor_map", "map", "$LOOP a[i] = b[i] ^ c[i];", _int, "full"),
    Kernel("not_map", "map", "$LOOP a[i] = ~b[i];", _int, "full"),
    Kernel("neg_map", "map", "$LOOP a[i] = -b[i];"),
    Kernel("shl", "map", "$LOOP a[i] = b[i] << 2;", _uint, "full"),
    Kernel("shr", "map", "$LOOP a[i] = b[i] >> 3;", _int, "full"),
    Kernel("shr_var", "map", "$LOOP a[i] = b[i] >> (c[i] & 7);", _uint, "full"),
    Kernel("abs_map", "map", "$LOOP a[i] = b[i] < 0 ? -b[i] : b[i];", _signed_or_fp),
    Kernel("clamp", "map",
           "$LOOP a[i] = b[i] < ($T)-$K ? ($T)-$K : (b[i] > ($T)$K ? ($T)$K : b[i]);",
           _signed_or_fp, "full"),
    Kernel("clamp_u", "map", "$LOOP a[i] = b[i] > ($T)$K ? ($T)$K : b[i];", _uint, "full"),
    Kernel("minmap", "map", "$LOOP a[i] = b[i] < c[i] ? b[i] : c[i];", _any, "full"),
    Kernel("maxmap", "map", "$LOOP a[i] = b[i] > c[i] ? b[i] : c[i];", _any, "full"),
    Kernel("select", "map", "$LOOP a[i] = b[i] > ($T)$K ? c[i] : b[i];", _any, "full"),
    Kernel("masked_store", "map", "$LOOP if (b[i] > ($T)$K) a[i] = c[i];", _any, "full"),
    Kernel("cond_inc", "map", "$LOOP a[i] = ($T)(($U)a[i] + ($U)(b[i] > ($T)$K));", _int, "full"),
    Kernel("fill", "map", "$LOOP a[i] = ($T)$K;", _any, "full"),
    Kernel("reverse", "map", "$LOOP a[i] = b[n - 1 - (long)i];", _any, "full"),
    Kernel("interleave", "map", "$LOOP { a[2 * i] = b[i]; a[2 * i + 1] = c[i]; }", _any, "full"),
    Kernel("deinterleave", "map", "$LOOP a[i] = b[2 * i] + b[2 * i + 1];"),
    Kernel("stencil3", "map",
           "for (long i = 1; i + 1 < n; i++) a[i] = b[i - 1] + b[i] + b[i + 1];",
           _any, "small", "wide", False),
    Kernel("stencil_min", "map",
           "for (long i = 1; i + 1 < n; i++) { $T m = b[i-1] < b[i+1] ? b[i-1] : b[i+1]; a[i] = m < b[i] ? m : b[i]; }",
           _any, "full", "wide", False),
    Kernel("gather", "map", "$LOOP a[i] = b[idx[i]];", _any, "full", "wide", True, False),
    Kernel("scatter_perm", "map", "$LOOP a[pidx[i]] = b[i];", _any, "full", "wide", True, False),
    Kernel("scatter_dup", "map", "$LOOP a[idx[i]] = b[i];", _any, "full", "wide", True, False),
    Kernel("histogram", "map", "$LOOP a[b[i] & 15]++;", _int, "full", "wide", True, False),
    # ── compare results used as VALUES (0/1 in scalar C, all-ones in SIMD) ──
    # A packed compare yields -1/0 per lane while C's `a > b` is 0/1; every
    # consumer that is not a select/blend condition must demask first.
    Kernel("cmp_store", "map", "$LOOP a[i] = b[i] > ($T)$K;", _int, "full"),
    Kernel("cmp_add2", "map", "$LOOP a[i] = (b[i] > ($T)$K) + (c[i] > ($T)$K);", _int, "full"),
    Kernel("cmp_sub", "map", "$LOOP a[i] = ($T)(($U)b[i] - ($U)(c[i] > ($T)$K));", _int, "full"),
    Kernel("cmp_mul", "map", "$LOOP a[i] = b[i] * (c[i] > ($T)$K);", _int, "small"),
    Kernel("cmp_and_val", "map", "$LOOP a[i] = b[i] & (c[i] > ($T)$K);", _int, "full"),
    Kernel("cmp_or_val", "map", "$LOOP a[i] = b[i] | (c[i] > ($T)$K);", _int, "full"),
    Kernel("cmp_xor_val", "map", "$LOOP a[i] ^= (b[i] == c[i]);", _int, "full"),
    Kernel("cmp_xor2", "map", "$LOOP a[i] = (b[i] > ($T)$K) ^ (c[i] < ($T)$K);", _int, "full"),
    Kernel("cmp_and2", "map", "$LOOP a[i] = (b[i] > ($T)$K) & (c[i] < ($T)$K);", _int, "full"),
    Kernel("cmp_or2", "map", "$LOOP a[i] = (b[i] > ($T)$K) | (c[i] < ($T)$K);", _int, "full"),
    Kernel("cmp_neg", "map", "$LOOP a[i] = -(b[i] > ($T)$K);", _int, "full"),
    Kernel("cmp_not", "map", "$LOOP a[i] = !(b[i] == ($T)$K);", _int, "full"),
    Kernel("cmp_arm", "map", "$LOOP a[i] = c[i] > ($T)$K ? (b[i] > ($T)$K) : 2;", _int, "full"),
    Kernel("cmp_arm2", "map", "$LOOP a[i] = c[i] > ($T)$K ? 2 : (b[i] == c[i]);", _int, "full"),
    Kernel("cmp_shl", "map", "$LOOP a[i] = (b[i] > ($T)$K) << 2;", _int, "full"),
    Kernel("cmp_acc_map", "map",
           "$LOOP a[i] = ($T)(($U)a[i] + ($U)(b[i] > ($T)$K) + ($U)(b[i] < ($T)-$K));", _int, "full"),
    # ── loop-carried recurrences (must NOT be vectorized naively) ──────────
    Kernel("prefix", "rec", "for (long i = 1; i < n; i++) a[i] = a[i - 1] + b[i];",
           _any, "small", "wide", False, True),
    Kernel("propagate", "rec", "for (long i = 0; i + 1 < n; i++) a[i + 1] = a[i];",
           _any, "full", "wide", False, True),
    Kernel("shift_down", "rec", "for (long i = 0; i + 1 < n; i++) a[i] = a[i + 1];",
           _any, "full", "wide", False, True),
    Kernel("recur2", "rec", "for (long i = 2; i < n; i++) a[i] = a[i - 2] + b[i];",
           _any, "small", "wide", False, True),
]

# (src, dst) element pairs for conversion kernels.
CONV_PAIRS = [("u8", "i32"), ("i8", "i32"), ("u8", "u16"), ("i8", "i16"), ("u16", "i32"),
              ("i16", "i32"), ("u16", "u32"), ("i32", "i64"), ("u32", "u64"), ("i32", "i8"),
              ("i32", "i16"), ("i64", "i32"), ("u32", "u8"), ("i32", "f32"), ("i32", "f64"),
              ("f32", "f64"), ("f64", "f32"), ("f32", "i32"), ("f64", "i32"), ("u8", "f32"),
              ("i16", "f32"), ("u32", "f64")]

# ── Program generation ───────────────────────────────────────────────────────

HEADER = """#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <stddef.h>
#define NBUF %d
#define PAD %d
""" % (NBUF, PAD)

LCG = "r = r * 6364136223846793005ULL + 1442695040888963407ULL;"


def fill_code(e: Elem, prof: str, name: str) -> str:
    """C statements filling buffer `name` (PAD+NBUF+PAD elements) per profile."""
    T = e.c
    loop = f"for (size_t k = 0; k < NBUF + 2 * PAD; k++) {{ {LCG} "
    if e.fp:
        if prof == "fp_adv":
            # Wildly different magnitudes: any reassociation changes the bits.
            val = (f"((r & 3) == 0 ? ({T})(1e8 * (double)((r >> 8) % 7 + 1)) "
                   f": ({T})((double)((r >> 8) % 1000) / 7.0))")
        else:
            val = f"({T})((int)((r >> 8) % 17) - 8)"
        return f"{loop}{name}[k] = {val}; }}"
    if prof == "full":
        ext = {8: "0x7f", 16: "0x7fff", 32: "0x7fffffff", 64: "0x7fffffffffffffff"}[e.bits]
        special = (
            f"if ((k % 19) == 0) {name}[k] = ({T})0; else if ((k % 23) == 1) {name}[k] = ({T})-1; "
            f"else if ((k % 29) == 2) {name}[k] = ({T})({ext}); "
            f"else if ((k % 31) == 3) {name}[k] = ({T})(-{ext} - 1);")
        return f"{loop}{name}[k] = ({T})(r >> 24); {special} }}"
    val = f"({T})((int)((r >> 24) % 17) - 8)" if e.signed else f"({T})((r >> 24) % 17)"
    return f"{loop}{name}[k] = {val}; }}"


def loop_head(form: str, ivt: str) -> str:
    if form == "lt":
        return f"for ({ivt} i = 0; i < ({ivt})n; i++)"
    if form == "start1":
        return f"for ({ivt} i = 1; i < ({ivt})n; i++)"
    if form == "step2":
        return f"for ({ivt} i = 0; i < ({ivt})n; i += 2)"
    if form == "down":
        return f"for ({ivt} i = ({ivt})n; i-- > 0;)"
    raise ValueError(form)


def expand(k: Kernel, e: Elem, form: str, ivt: str) -> str:
    acc = e.c if k.acc == "T" else e.wide
    if e.fp:
        mx, mn = ("3.4e38f", "-3.4e38f") if e.bits == 32 else ("1.7e308", "-1.7e308")
    else:
        mx = {8: "INT8_MAX", 16: "INT16_MAX", 32: "INT32_MAX", 64: "INT64_MAX"}[e.bits] if e.signed else \
            {8: "UINT8_MAX", 16: "UINT16_MAX", 32: "UINT32_MAX", 64: "UINT64_MAX"}[e.bits]
        mn = {8: "INT8_MIN", 16: "INT16_MIN", 32: "INT32_MIN", 64: "INT64_MIN"}[e.bits] if e.signed else "0"
    kmid = "3" if (e.bits <= 8 and not e.signed) or e.fp else "5"
    body = k.body.replace("$LOOP", loop_head(form, ivt) if k.loop else "")
    return (body.replace("$T", e.c).replace("$U", e.unsigned_c).replace("$A", acc).replace("$MAX", f"({e.c}){mx}")
            .replace("$MIN", f"({e.c}){mn}").replace("$K", kmid))


def gen_program(k: Kernel, e: Elem, form: str, ivt: str) -> str:
    """One self-contained C program exercising kernel `k` at element type `e`."""
    T = e.c
    body = expand(k, e, form, ivt)
    returns = "return" in body
    if not returns:
        ret_t = "void"
    elif e.fp and k.acc == "T":
        ret_t = "double"
    else:
        ret_t = "long long"
    sig = f"{T} *a, const {T} *b, const {T} *c, const {T} *c2, const int *idx, const int *pidx, long n"
    uses_idx = "idx[" in body
    call_ret = {"void": "", "double": "double rv = ", "long long": "long long rv = "}[ret_t]
    mix_ret = "" if ret_t == "void" else "mix(&rv, sizeof rv);"
    trips = ", ".join(str(t) for t in TRIPS)
    placements = ", ".join("{%d,%d,%d}" % p for p in PLACEMENTS)
    aliases = ", ".join(str(a) for a in ALIAS_OFFSETS)
    alias_block = ""
    if k.kind in ("map", "rec") and k.alias and not uses_idx:
        # Overlapping source/destination: b aliases a (and c aliases a at a
        # second offset) -- the dependence analysis must be exact.
        alias_block = f"""    for (size_t ai = 0; ai < sizeof al / sizeof *al; ai++) {{
      memcpy(A, A0, sizeof A);
      {T} *p = A + PAD + 16;
      kernel(p + al[ai], p, p + (al[ai] < 0 ? 2 : -2), C2, IDX + PAD, PIDX + PAD, n);
      mix(A, sizeof A);
    }}
"""
    return HEADER + f"""
__attribute__((noinline)) {ret_t} kernel({sig}) {{
  (void)a; (void)b; (void)c; (void)c2; (void)idx; (void)pidx;
  {body}
}}
static uint64_t h = 1469598103934665603ULL;
static void mix(const void *p, size_t n) {{
  const unsigned char *q = p;
  for (size_t k = 0; k < n; k++) {{ h ^= q[k]; h *= 1099511628211ULL; }}
}}
static {T} A[NBUF + 2 * PAD], B[NBUF + 2 * PAD], C[NBUF + 2 * PAD], A0[NBUF + 2 * PAD], C2[8];
static int IDX[NBUF + 2 * PAD], PIDX[NBUF + 2 * PAD];
static uint64_t r = 88172645463325252ULL;

int main(void) {{
  static const int trips[] = {{{trips}}};
  static const int pl[][3] = {{{placements}}};
  static const int al[] = {{{aliases}}};
  {fill_code(e, k.profile, "B")}
  {fill_code(e, k.profile, "C")}
  {fill_code(e, k.profile, "A0")}
  for (int k = 0; k < NBUF + 2 * PAD; k++) {{
    {LCG}
    IDX[k] = (int)((r >> 33) % 97);
  }}
  /* permutation of [0, 500): stride 7 is coprime with 500 */
  for (int k = 0; k < 500; k++) PIDX[PAD + k] = (k * 7 + 3) % 500;
  for (int k = 0; k < 8; k++) C2[k] = C[PAD + 3 + k];
  for (size_t ti = 0; ti < sizeof trips / sizeof *trips; ti++) {{
    long n = trips[ti];
    for (size_t pi = 0; pi < sizeof pl / sizeof *pl; pi++) {{
      memcpy(A, A0, sizeof A);
      {call_ret}kernel(A + PAD + pl[pi][0], B + PAD + pl[pi][1], C + PAD + pl[pi][2], C2,
                       IDX + PAD, PIDX + PAD, n);
      mix(A, sizeof A); {mix_ret}
    }}
{alias_block}  }}
  printf("%016llx\\n", (unsigned long long)h);
  return 0;
}}
"""


def gen_conv_program(src: Elem, dst: Elem) -> str:
    """Conversion kernels: a[i] = (DST)b[i]; plus a widening-accumulate form."""
    S, D = src.c, dst.c
    fp = src.fp or dst.fp
    # FP<->int needs in-range inputs to stay defined; int narrowing is a
    # defined (unsigned) / wrapping (signed, GCC-documented) truncation.
    prof = "small" if fp else "full"
    acc_t = "double" if fp else "long long"
    trips = ", ".join(str(t) for t in TRIPS)
    return HEADER + f"""
__attribute__((noinline)) void kernel({D} *a, const {S} *b, long n) {{
  for (long i = 0; i < n; i++) a[i] = ({D})b[i];
}}
__attribute__((noinline)) {acc_t} kernel_acc(const {S} *b, long n) {{
  {acc_t} s = 0;
  for (long i = 0; i < n; i++) s += ({acc_t})({D})b[i];
  return s;
}}
static uint64_t h = 1469598103934665603ULL;
static void mix(const void *p, size_t n) {{
  const unsigned char *q = p;
  for (size_t k = 0; k < n; k++) {{ h ^= q[k]; h *= 1099511628211ULL; }}
}}
static {S} B[NBUF + 2 * PAD];
static {D} A[NBUF + 2 * PAD];
static uint64_t r = 88172645463325252ULL;
int main(void) {{
  static const int trips[] = {{{trips}}};
  {fill_code(src, prof, "B")}
  for (size_t ti = 0; ti < sizeof trips / sizeof *trips; ti++) {{
    for (int off = 0; off < 4; off++) {{
      memset(A, 0x5a, sizeof A);
      kernel(A + PAD + off, B + PAD + (off ^ 1), trips[ti]);
      mix(A, sizeof A);
      {acc_t} s = kernel_acc(B + PAD + off, trips[ti]);
      mix(&s, sizeof s);
    }}
  }}
  printf("%016llx\\n", (unsigned long long)h);
  return 0;
}}
"""


# ── Case enumeration ─────────────────────────────────────────────────────────


@dataclass
class Case:
    cid: str
    src: str
    kernel: str


def enumerate_cases(args: argparse.Namespace) -> List[Case]:
    rng = random.Random(args.seed)
    cases: List[Case] = []
    flt = re.compile(args.filter) if args.filter else None
    for k in KERNELS:
        for e in ELEMS:
            if not k.types(e):
                continue
            if flt and not flt.search(f"{k.name}_{e.tag}"):
                continue
            if not k.loop:
                forms = [("lt", "int")]
            elif args.exhaustive_forms:
                forms = [(f, t) for f in FORMS for t in IV_TYPES
                         if not (f == "down" and t in ("unsigned", "size_t"))]
            else:
                # Deterministic rotation: every (form, ivt) pair is hit across
                # the kernel set; `--seed` reshuffles.  The canonical
                # `int`/`lt` spelling is always included.
                f = rng.choice(FORMS)
                t = rng.choice([t for t in IV_TYPES if not (f == "down" and t in ("unsigned", "size_t"))])
                forms = [("lt", "int")] + ([(f, t)] if (f, t) != ("lt", "int") else [])
            for f, t in forms:
                cases.append(Case(f"{k.name}_{e.tag}_{f}_{t}", gen_program(k, e, f, t), k.name))
    if not args.no_conv:
        for s, d in CONV_PAIRS:
            if flt and not flt.search(f"conv_{s}_{d}"):
                continue
            cases.append(Case(f"conv_{s}_{d}", gen_conv_program(BY_TAG[s], BY_TAG[d]), "conv"))
    if args.limit:
        rng.shuffle(cases)
        cases = cases[: args.limit]
    return cases


# ── Execution ────────────────────────────────────────────────────────────────

# Packed-SIMD census classifier.  Operand-aware: whether an instruction is a
# real vector operation depends on its OPERANDS, not just its mnemonic --
# `pxor %xmm0,%xmm0` is the zeroing idiom but `pxor (%rax),%xmm0` (or two
# distinct registers) is the kernel's actual XOR; `movdqu (%rsi),%xmm0` is a
# vector load but `movaps %xmm1,%xmm0` is a register copy.
PACKED_MN = re.compile(
    r"^(v?p[a-z0-9]+|v[a-z0-9]+p[sd]|[a-z0-9]+p[sd]|v?cvt[a-z0-9]*|vbroadcast\w+|vinserti\w+|vextracti\w+)$")
# Scalar FP arithmetic/moves/compares on the low lane (`*ss` / `*sd`).  Listed
# explicitly: a bare "ends in sd" test also matches packed integer ops such as
# `vpabsd` / `vpmaxsd`.
SCALAR_FP = re.compile(
    r"^v?(?:add|sub|mul|div|min|max|sqrt|rcp|rsqrt|mov|round|ucomi|comi|cmp[a-z]*)s[sd]$"
    r"|^vf[n]?m(?:add|sub)\d{3}s[sd]$")
# Scalar int<->FP conversions (`cvtsi2sd`, `cvttsd2si`, `cvtss2sd`, ...).
SCALAR_CVT = re.compile(r"^v?cvtt?s[sdi]2|^v?cvtt?\w*2s[sdi]$")
# GPR <-> vector scalar moves and ymm bookkeeping.
NON_SIMD_MN = {"movd", "movq", "vmovd", "vmovq", "vzeroupper", "vzeroall", "emms"}
# Packed (whole-register) loads/stores/moves: SIMD only when one side is memory.
PACKED_MOVE = re.compile(
    r"^v?(?:movdq[au]|movap[sd]|movup[sd]|lddqu|movnt(?:dq|ps|pd)a?)(?:8|16|32|64)?$"
    r"|^vmovdq[au](?:8|16|32|64)$")
# Idioms that read no data: dst = f(src, src) with identical register operands.
SELF_IDIOM = re.compile(r"^v?(?:pxor|xorp[sd]|psub[bwdq]|pandn|pcmpeq[bwdq])$")


def split_operands(ops: str) -> List[str]:
    """Split an AT&T operand list on top-level commas (not those inside `(...)`)."""
    out, depth, cur = [], 0, []
    for ch in ops.split("#")[0].strip():
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
        if ch == "," and depth == 0:
            out.append("".join(cur).strip())
            cur = []
        else:
            cur.append(ch)
    if cur:
        out.append("".join(cur).strip())
    return [o for o in out if o]


def is_packed_simd(mn: str, ops: str) -> bool:
    """Is `mn ops` a real packed-SIMD data operation (see the block comment)?"""
    operands = split_operands(ops)
    vec = [o for o in operands if re.fullmatch(r"%[xyz]mm\d+(?:\{[^}]*\})*", o)]
    if not vec or mn in NON_SIMD_MN:
        return False
    if PACKED_MOVE.match(mn):
        return any("(" in o or o.startswith(("0x", "-", "%fs", "%gs")) for o in operands)
    if SELF_IDIOM.match(mn) and len(set(o for o in operands if not o.startswith("$"))) == 1 \
            and all(o in vec for o in operands if not o.startswith("$")):
        return False  # zero / all-ones idiom: no data read
    if SCALAR_CVT.match(mn) or SCALAR_FP.match(mn):
        return False
    return bool(PACKED_MN.match(mn))


def count_simd_in_disassembly(text: str) -> int:
    """Packed-SIMD instructions in `objdump -d --no-show-raw-insn` output."""
    n = 0
    for line in text.splitlines():
        m = re.match(r"^\s+[0-9a-f]+:\s+(\S+)\s*(.*)$", line)
        if m and is_packed_simd(m.group(1), m.group(2)):
            n += 1
    return n


def sh(cmd: Sequence[str], env: Optional[dict] = None, timeout: int = 180) -> subprocess.CompletedProcess:
    return subprocess.run(list(cmd), capture_output=True, text=True, timeout=timeout, env=env)


def compile_run(compiler: str, flags: Sequence[str], src: Path, exe: Path,
                env_extra: Optional[dict]) -> Tuple[str, str]:
    env = dict(os.environ)
    if env_extra:
        env.update(env_extra)
    try:
        c = sh([compiler, *flags, str(src), "-o", str(exe)], env=env)
    except subprocess.TimeoutExpired:
        return "COMPILE-TIMEOUT", ""
    if c.returncode != 0:
        return "COMPILE-FAIL", (c.stderr or c.stdout)[-400:]
    try:
        r = sh([str(exe)], timeout=60)
    except subprocess.TimeoutExpired:
        return "RUN-TIMEOUT", ""
    if r.returncode != 0:
        return f"RUN-FAIL({r.returncode})", (r.stderr or "")[-200:]
    return "OK", r.stdout.strip()


def simd_count(obj: Path, symbol: str = "kernel") -> Optional[int]:
    """Packed-SIMD instructions inside `symbol` (see `is_packed_simd`)."""
    d = sh(["objdump", "-d", "--no-show-raw-insn", f"--disassemble={symbol}", str(obj)])
    if d.returncode != 0:
        return None
    return count_simd_in_disassembly(d.stdout)


def run_case(case: Case, args: argparse.Namespace, workdir: Path) -> dict:
    d = workdir / case.cid
    d.mkdir(parents=True, exist_ok=True)
    src = d / "t.c"
    src.write_text(case.src)
    lflags = shlex.split(args.lccc_flags)
    res: Dict[str, Tuple[str, str]] = {
        "gcc-O0": compile_run(args.gcc, ["-O0", "-w"], src, d / "ref", None),
        "lccc-on": compile_run(args.lccc, lflags, src, d / "on", None),
        "lccc-off": compile_run(args.lccc, lflags, src, d / "off",
                                {"CCC_DISABLE_PASSES": "vectorize,slp,slp_late,iv_widen"}),
    }
    status, detail = "PASS", ""
    ref = res["gcc-O0"]
    if ref[0] != "OK":
        status, detail = "REF-FAIL", f"gcc -O0: {ref}"
    else:
        for arm in ("lccc-on", "lccc-off"):
            if res[arm] != ref:
                status = "MISCOMPILE" if res[arm][0] == "OK" else "FAIL"
                detail += f"{arm}: {res[arm][0]} {res[arm][1]!r} vs ref {ref[1]!r}; "
    census = None
    if args.census and status == "PASS":
        lo, go = d / "l.o", d / "g.o"
        lc = sh([args.lccc, *lflags, "-c", str(src), "-o", str(lo)])
        gflags = [f for f in lflags if f.startswith(("-march", "-mavx", "-ffp-contract"))]
        gc = sh([args.gcc, "-O3", "-w", *gflags, "-c", str(src), "-o", str(go)])
        if lc.returncode == 0 and gc.returncode == 0:
            census = {"lccc": simd_count(lo), "gcc": simd_count(go)}
    if status != "PASS" and args.keep:
        dst = Path(args.keep) / case.cid
        shutil.copytree(d, dst, dirs_exist_ok=True, ignore=shutil.ignore_patterns("ref", "on", "off", "*.o"))
    if status == "PASS" and args.keep_all and args.keep:
        shutil.copytree(d, Path(args.keep) / case.cid, dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns("ref", "on", "off", "*.o"))
    shutil.rmtree(d, ignore_errors=True)
    return {"id": case.cid, "kernel": case.kernel, "status": status, "detail": detail, "census": census}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--lccc", default="target/fastbuild/lccc-x86")
    ap.add_argument("--gcc", default=os.environ.get("CC", "gcc"))
    # -ffp-contract=off: lccc (like gcc -march=x86-64-v3) contracts a*b+c into FMA
    # by default, which legitimately changes FP bits versus the -O0 reference.
    # The stress test compares bits, so contraction is pinned off in every arm.
    ap.add_argument("--lccc-flags", default="-O3 -march=x86-64-v3 -ffp-contract=off")
    ap.add_argument("--filter", help="regex on <kernel>_<type> (conv_<src>_<dst> for conversions)")
    ap.add_argument("--limit", type=int, default=0, help="random subset of N cases (with --seed)")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--exhaustive-forms", action="store_true",
                    help="every (loop form x IV type) pair per kernel/type (large)")
    ap.add_argument("--no-conv", action="store_true")
    ap.add_argument("--census", action="store_true",
                    help="also rank kernels where gcc -O3 vectorizes and lccc does not")
    ap.add_argument("--json", help="write full results to this file")
    ap.add_argument("--keep", help="copy failing reproducers into this directory")
    ap.add_argument("--keep-all", action="store_true", help="with --keep: also keep passing sources")
    ap.add_argument("-j", "--jobs", type=int, default=2)
    args = ap.parse_args()

    cases = enumerate_cases(args)
    if not cases:
        print("no cases matched", file=sys.stderr)
        return 2
    print(f"vectorize_stress: {len(cases)} programs, lccc flags: {args.lccc_flags}", flush=True)
    work = Path(tempfile.mkdtemp(prefix="vecstress-"))
    results: List[dict] = []
    try:
        with cf.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as ex:
            futs = [ex.submit(run_case, c, args, work) for c in cases]
            for done, f in enumerate(cf.as_completed(futs), 1):
                r = f.result()
                results.append(r)
                if r["status"] != "PASS":
                    print(f"  [{r['status']}] {r['id']}: {r['detail']}", flush=True)
                elif done % 50 == 0:
                    print(f"  ... {done}/{len(cases)}", flush=True)
    finally:
        shutil.rmtree(work, ignore_errors=True)

    bad = [r for r in results if r["status"] != "PASS"]
    print(f"\n{len(results) - len(bad)}/{len(results)} programs agree across all arms")
    if args.census:
        rows: Dict[str, Dict[str, int]] = {}
        for r in results:
            c = r.get("census")
            if not c or c["lccc"] is None or c["gcc"] is None:
                continue
            m = rows.setdefault(r["kernel"], {"n": 0, "gcc": 0, "lccc": 0, "gap": 0})
            m["n"] += 1
            m["gcc"] += c["gcc"] > 0
            m["lccc"] += c["lccc"] > 0
            m["gap"] += c["gcc"] > 0 and c["lccc"] == 0
        print("\nMissed-vectorization census (gcc -O3 emits packed SIMD in `kernel`, lccc does not):")
        print(f"{'kernel':<16} {'n':>4} {'gcc-vec':>8} {'lccc-vec':>9} {'gap':>4}")
        for name, m in sorted(rows.items(), key=lambda kv: (-kv[1]["gap"], kv[0])):
            print(f"{name:<16} {m['n']:>4} {m['gcc']:>8} {m['lccc']:>9} {m['gap']:>4}")
    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(sorted(results, key=lambda r: r["id"]), indent=1))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
