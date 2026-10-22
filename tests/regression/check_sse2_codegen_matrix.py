#!/usr/bin/env python3
"""SSE2 codegen matrix: one row per SSE/SSE2 instruction, LCCC vs both oracles.

WHY THIS EXISTS
---------------
`include/xmmintrin.h` and `include/emmintrin.h` give LCCC the whole SSE/SSE2
intrinsic surface, but nothing measured whether the *emitted code* for that
surface is competitive.  The existing gates answer adjacent questions: the
regression corpus checks results, `check_vectorize_isa_gate.sh` checks which
vectorizer arm runs, and the asm census counts mnemonics corpus-wide.  None of
them compares, per instruction, how many instructions LCCC needs against GCC
and Clang for the same source.  A kernel can be bit-exact and still need five
times the instruction count of both oracles -- which is exactly what a
memory-backed intrinsic ABI produces.

The matrix is built the way the compiler's ISA support is built: from a
declarative table, one row per instruction.  Each row names the mnemonic it is
about, the intrinsic sequence that should lower to it, and its operand class.
For each row and compiler the harness:

  1. compiles the row's kernel at the requested optimization configs,
  2. extracts that kernel's instruction stream from the emitted assembly,
  3. censuses instructions (count and per-mnemonic tally),
  4. runs the kernel and hashes the raw output bytes, and
  5. in `--mode timing`, measures CPU time per call.

WHAT IT PROVES
--------------
Three independent facts, each able to fail on its own:

* **Lowering fidelity** -- the row's expected mnemonic must appear in the
  kernel body.  This catches a missed single-instruction mapping (an
  `unpcklpd` pair where `movlhps` was meant, a `pand`/`pandn` expansion, a
  scalarized packed op).
* **Differential correctness** -- LCCC's output hash must equal *both* oracles'
  hashes, bit for bit.  Kernels are written so results are exact in binary
  floating point (integer-valued doubles, integer SIMD), never accidental.
* **Instruction economy** -- LCCC's instruction count must not exceed the best
  oracle's for any row in the parity set.  Rows that exceed it are recorded in
  the committed baseline's `debt` map with their measured gap; the gate fails
  if a debt gap *grows* and if a parity row starts losing.  The gate is
  therefore monotone: it can only tighten.

The baseline file is the data.  `--update-baseline` rewrites it and
`--report` prints the table, so every claim in
`engineering/SSE2-CODEGEN-MATRIX.md` is reproducible from a command line
rather than asserted in prose.

USAGE
-----
    python3 tests/regression/check_sse2_codegen_matrix.py --check
    python3 tests/regression/check_sse2_codegen_matrix.py --report
    python3 tests/regression/check_sse2_codegen_matrix.py --mode timing --report
    python3 tests/regression/check_sse2_codegen_matrix.py --update-baseline

Compilers default to `target/fastbuild/lccc` and the environment variables
`GCC`/`CLANG`.  An unavailable oracle is reported as skipped, never as a pass.
x86-64 only: the subject is the SSE/SSE2 encoding.
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).resolve().parent / "sse2_codegen_matrix.json"

DEFAULT_CONFIGS = ("O2",)


# ---------------------------------------------------------------------------
# The table: one entry per SSE/SSE2 instruction.
#
#   mnemonic : the instruction the row is about
#   cls      : operand class, which selects the kernel's parameter types
#   body     : the loop body, verbatim C
#   expect   : mnemonics that must appear in the emitted kernel body
#
# `cls` decides what `x`, `y` and `R` are:
#   "d"  x,y: __m128d   loaded from a,b (double arrays)
#   "f"  x,y: __m128    loaded from a,b (float arrays)
#   "i"  x,y: __m128i   loaded from a,b (long long arrays)
#   "sc" x,y: double    scalars from a,b (double arrays)
#   "x"  the body declares its own operands
# The result variable is always `R` and is stored to `o` unless the body does
# its own store (`/*own*/` marker).
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Row:
    mnemonic: str
    cls: str
    body: str
    expect: tuple[str, ...]
    note: str = ""

    @property
    def sym(self) -> str:
        return "k_" + re.sub(r"[^a-z0-9]+", "_", self.mnemonic).strip("_")


ROWS: list[Row] = []


def row(mnemonic: str, cls: str, body: str, expect=None, note: str = "") -> None:
    ROWS.append(Row(mnemonic, cls, body, tuple(expect or (mnemonic,)), note))


def binop(mn: str, intr: str, cls: str = "d") -> None:
    """`intr` is the complete expression; `x`/`y` are the loaded operands."""
    row(mn, cls, intr if "(" in intr else f"{intr}(x, y)")


def unop(mn: str, expr: str, cls: str = "d") -> None:
    row(mn, cls, expr)


# --- packed double (SSE2) ---------------------------------------------------
for mn, intr in [
    ("addpd", "_mm_add_pd(x, y)"),
    ("subpd", "_mm_sub_pd(x, y)"),
    ("mulpd", "_mm_mul_pd(x, y)"),
    ("divpd", "_mm_div_pd(x, y)"),
    ("maxpd", "_mm_max_pd(x, y)"),
    ("minpd", "_mm_min_pd(x, y)"),
    ("andpd", "_mm_and_pd(x, y)"),
    ("andnpd", "_mm_andnot_pd(x, y)"),
    ("orpd", "_mm_or_pd(x, y)"),
    ("xorpd", "_mm_xor_pd(x, y)"),
]:
    binop(mn, intr)
unop("sqrtpd", "_mm_sqrt_pd(x)")
binop("shufpd", "_mm_shuffle_pd(x, y, 1)")
binop("unpcklpd", "_mm_unpacklo_pd(x, y)")
binop("unpckhpd", "_mm_unpackhi_pd(x, y)")
binop("cmpgtpd", "_mm_cmpgt_pd(x, y)")
binop("cmpeqpd", "_mm_cmpeq_pd(x, y)")
binop("cmpltpd", "_mm_cmplt_pd(x, y)")
row("movmskpd", "d", "_mm_castsi128_pd(_mm_set1_epi32(_mm_movemask_pd(_mm_cmpgt_pd(x, y))))")

# --- packed single (SSE) ---------------------------------------------------
for mn, intr in [
    ("addps", "_mm_add_ps(x, y)"),
    ("subps", "_mm_sub_ps(x, y)"),
    ("mulps", "_mm_mul_ps(x, y)"),
    ("divps", "_mm_div_ps(x, y)"),
    ("maxps", "_mm_max_ps(x, y)"),
    ("minps", "_mm_min_ps(x, y)"),
    ("andps", "_mm_and_ps(x, y)"),
    ("andnps", "_mm_andnot_ps(x, y)"),
    ("orps", "_mm_or_ps(x, y)"),
    ("xorps", "_mm_xor_ps(x, y)"),
]:
    binop(mn, intr, "f")
unop("sqrtps", "_mm_sqrt_ps(x)", "f")
binop("shufps", "_mm_shuffle_ps(x, y, 0x1B)", "f")
binop("unpcklps", "_mm_unpacklo_ps(x, y)", "f")
binop("unpckhps", "_mm_unpackhi_ps(x, y)", "f")
binop("cmpltps", "_mm_cmplt_ps(x, y)", "f")
binop("cmpgtps", "_mm_cmpgt_ps(x, y)", "f")
binop("cmpeqps", "_mm_cmpeq_ps(x, y)", "f")

# --- scalar double / single ------------------------------------------------
for mn, intr in [
    ("addsd", "_mm_add_sd(x, y)"),
    ("subsd", "_mm_sub_sd(x, y)"),
    ("mulsd", "_mm_mul_sd(x, y)"),
    ("divsd", "_mm_div_sd(x, y)"),
    ("maxsd", "_mm_max_sd(x, y)"),
    ("minsd", "_mm_min_sd(x, y)"),
]:
    binop(mn, intr)
unop("sqrtsd", "_mm_sqrt_sd(x, x)")
for mn, intr in [
    ("addss", "_mm_add_ss(x, y)"),
    ("subss", "_mm_sub_ss(x, y)"),
    ("mulss", "_mm_mul_ss(x, y)"),
    ("divss", "_mm_div_ss(x, y)"),
]:
    binop(mn, intr, "f")
unop("sqrtss", "_mm_sqrt_ss(x)", "f")

# --- integer SIMD: add/sub/saturating/avg ---------------------------------
for mn, intr in [
    ("paddb", "_mm_add_epi8(x, y)"),
    ("paddw", "_mm_add_epi16(x, y)"),
    ("paddd", "_mm_add_epi32(x, y)"),
    ("paddq", "_mm_add_epi64(x, y)"),
    ("psubb", "_mm_sub_epi8(x, y)"),
    ("psubw", "_mm_sub_epi16(x, y)"),
    ("psubd", "_mm_sub_epi32(x, y)"),
    ("psubq", "_mm_sub_epi64(x, y)"),
    ("paddsb", "_mm_adds_epi8(x, y)"),
    ("paddsw", "_mm_adds_epi16(x, y)"),
    ("paddusb", "_mm_adds_epu8(x, y)"),
    ("paddusw", "_mm_adds_epu16(x, y)"),
    ("psubsb", "_mm_subs_epi8(x, y)"),
    ("psubsw", "_mm_subs_epi16(x, y)"),
    ("psubusb", "_mm_subs_epu8(x, y)"),
    ("psubusw", "_mm_subs_epu16(x, y)"),
    ("pavgb", "_mm_avg_epu8(x, y)"),
    ("pavgw", "_mm_avg_epu16(x, y)"),
]:
    binop(mn, intr, "i")

# --- integer SIMD: bitwise / compare / min / max ---------------------------
for mn, intr in [
    ("pand", "_mm_and_si128(x, y)"),
    ("pandn", "_mm_andnot_si128(x, y)"),
    ("por", "_mm_or_si128(x, y)"),
    ("pxor", "_mm_xor_si128(x, y)"),
    ("pcmpeqb", "_mm_cmpeq_epi8(x, y)"),
    ("pcmpeqw", "_mm_cmpeq_epi16(x, y)"),
    ("pcmpeqd", "_mm_cmpeq_epi32(x, y)"),
    ("pcmpgtb", "_mm_cmpgt_epi8(x, y)"),
    ("pcmpgtw", "_mm_cmpgt_epi16(x, y)"),
    ("pcmpgtd", "_mm_cmpgt_epi32(x, y)"),
    ("pmaxub", "_mm_max_epu8(x, y)"),
    ("pminsw", "_mm_min_epi16(x, y)"),
]:
    binop(mn, intr, "i")

# --- integer SIMD: multiply / pack / unpack --------------------------------
for mn, intr in [
    ("pmullw", "_mm_mullo_epi16(x, y)"),
    ("pmulhw", "_mm_mulhi_epi16(x, y)"),
    ("pmulhuw", "_mm_mulhi_epu16(x, y)"),
    ("pmuludq", "_mm_mul_epu32(x, y)"),
    ("pmaddwd", "_mm_madd_epi16(x, y)"),
    ("psadbw", "_mm_sad_epu8(x, y)"),
    ("packsswb", "_mm_packs_epi16(x, y)"),
    ("packssdw", "_mm_packs_epi32(x, y)"),
    ("packuswb", "_mm_packus_epi16(x, y)"),
    ("punpcklbw", "_mm_unpacklo_epi8(x, y)"),
    ("punpcklwd", "_mm_unpacklo_epi16(x, y)"),
    ("punpckldq", "_mm_unpacklo_epi32(x, y)"),
    ("punpcklqdq", "_mm_unpacklo_epi64(x, y)"),
    ("punpckhbw", "_mm_unpackhi_epi8(x, y)"),
    ("punpckhwd", "_mm_unpackhi_epi16(x, y)"),
    ("punpckhdq", "_mm_unpackhi_epi32(x, y)"),
    ("punpckhqdq", "_mm_unpackhi_epi64(x, y)"),
]:
    binop(mn, intr, "i")

# --- integer SIMD: shuffles and shifts -------------------------------------
unop("pshufd", "_mm_shuffle_epi32(x, 0x1B)", "i")
unop("pshufhw", "_mm_shufflehi_epi16(x, 0x1B)", "i")
unop("pshuflw", "_mm_shufflelo_epi16(x, 0x1B)", "i")
unop("psllw", "_mm_slli_epi16(x, 3)", "i")
unop("pslld", "_mm_slli_epi32(x, 3)", "i")
unop("psllq", "_mm_slli_epi64(x, 3)", "i")
unop("psrlw", "_mm_srli_epi16(x, 3)", "i")
unop("psrld", "_mm_srli_epi32(x, 3)", "i")
unop("psrlq", "_mm_srli_epi64(x, 3)", "i")
unop("psraw", "_mm_srai_epi16(x, 3)", "i")
unop("psrad", "_mm_srai_epi32(x, 3)", "i")
unop("pslldq", "_mm_slli_si128(x, 4)", "i")
unop("psrldq", "_mm_srli_si128(x, 4)", "i")

# --- conversions -----------------------------------------------------------
row("cvtdq2pd", "raw", "{ __m128d r = _mm_cvtepi32_pd(_mm_loadu_si128((const __m128i *)((const char *)b + i))); _mm_storeu_pd((double *)((char *)o + i), r); }", ("cvtdq2pd",))
row("cvtpd2dq", "raw", "{ __m128i r = _mm_cvtpd_epi32(_mm_loadu_pd((const double *)((const char *)a + i))); _mm_storeu_si128((__m128i *)((char *)o + i), r); }", ("cvtpd2dq",))
row("cvttpd2dq", "raw", "{ __m128i r = _mm_cvttpd_epi32(_mm_loadu_pd((const double *)((const char *)a + i))); _mm_storeu_si128((__m128i *)((char *)o + i), r); }", ("cvttpd2dq",))
row("cvtdq2ps", "raw", "{ __m128 r = _mm_cvtepi32_ps(_mm_loadu_si128((const __m128i *)((const char *)b + i))); _mm_storeu_ps((float *)((char *)o + i), r); }", ("cvtdq2ps",))
row("cvtps2dq", "raw", "{ __m128i r = _mm_cvtps_epi32(_mm_loadu_ps((const float *)((const char *)b + i))); _mm_storeu_si128((__m128i *)((char *)o + i), r); }", ("cvtps2dq",))
row("cvttps2dq", "raw", "{ __m128i r = _mm_cvttps_epi32(_mm_loadu_ps((const float *)((const char *)b + i))); _mm_storeu_si128((__m128i *)((char *)o + i), r); }", ("cvttps2dq",))
row("cvtsd2ss", "raw", "{ __m128 r = _mm_cvtpd_ps(_mm_loadu_pd((const double *)((const char *)a + i))); _mm_storeu_ps((float *)((char *)o + i), r); }", ("cvtpd2ps",))
row("cvtss2sd", "raw", "{ __m128d r = _mm_cvtps_pd(_mm_loadu_ps((const float *)((const char *)b + i))); _mm_storeu_pd((double *)((char *)o + i), r); }", ("cvtps2pd",))
row("cvtsd2si", "raw", "{ int r = _mm_cvtsd_si32(_mm_loadu_pd((const double *)((const char *)a + i))); _mm_storeu_si128((__m128i *)((char *)o + i), _mm_set1_epi32(r)); }", ("cvtsd2si",))
row("cvttsd2si", "raw", "{ int r = _mm_cvttsd_si32(_mm_loadu_pd((const double *)((const char *)a + i))); _mm_storeu_si128((__m128i *)((char *)o + i), _mm_set1_epi32(r)); }", ("cvttsd2si",))
row("cvtsi2sd", "raw", "{ __m128d r = _mm_cvtsi64_sd(_mm_setzero_pd(), (long long)i); _mm_storeu_pd((double *)((char *)o + i), r); }", ("cvtsi2sd",))
row("cvtsi2ss", "raw", "{ __m128 r = _mm_cvtsi32_ss(_mm_setzero_ps(), (int)i); _mm_storeu_ps((float *)((char *)o + i), r); }", ("cvtsi2ss",))

# --- memory forms ----------------------------------------------------------
row("movapd", "d", "x", ("movapd", "movupd"), "aligned/unaligned 128-bit copy")
row("movdqa", "i", "x", ("movdqa", "movdqu"), "aligned/unaligned integer copy")

# --- the shape that decides real runtime -----------------------------------
row("matmul_ikj", "x", "", ("mulpd", "addpd"), "vectorizer arm (loop-nest kernel)")


KERNEL_PRELUDE = """\
#include <emmintrin.h>
#include <xmmintrin.h>
#include <stdio.h>

static unsigned long long fnv1a(const unsigned char *p, unsigned long n)
{
    unsigned long long h = 1469598103934665603ULL;
    for (unsigned long i = 0; i < n; i++) {
        h ^= p[i];
        h *= 1099511628211ULL;
    }
    return h;
}
"""

MATMUL = """
#define MTX_N 48
static double MA[MTX_N][MTX_N], MB[MTX_N][MTX_N], MC[MTX_N][MTX_N];
__attribute__((noinline)) void k_matmul_ikj(const void *a, const void *b, void *o, int n)
{
    (void)a; (void)b; (void)o; (void)n;
    for (int i = 0; i < MTX_N; i++)
        for (int k = 0; k < MTX_N; k++)
            for (int j = 0; j < MTX_N; j++)
                MC[i][j] += MA[i][k] * MB[k][j];
}
"""

CLASS_DECLS = {
    "d": ('__m128d x = _mm_loadu_pd((const double *)((const char *)a + i));',
          '__m128d y = _mm_loadu_pd((const double *)((const char *)b + i));',
          '__m128d R;', '_mm_storeu_pd((double *)((char *)o + i), R);'),
    "f": ('__m128 x = _mm_loadu_ps((const float *)((const char *)a + i));',
          '__m128 y = _mm_loadu_ps((const float *)((const char *)b + i));',
          '__m128 R;', '_mm_storeu_ps((float *)((char *)o + i), R);'),
    "i": ('__m128i x = _mm_loadu_si128((const __m128i *)((const char *)a + i));',
          '__m128i y = _mm_loadu_si128((const __m128i *)((const char *)b + i));',
          '__m128i R;', '_mm_storeu_si128((__m128i *)((char *)o + i), R);'),
}


def kernel_source(rows: list[Row], solo: bool = False) -> str:
    out = [KERNEL_PRELUDE]
    for r in rows:
        if r.mnemonic == "matmul_ikj":
            out.append(MATMUL)
            continue
        if r.cls == "raw":
            out.append(
                f"""\
__attribute__((noinline)) void {r.sym}(const void *restrict a, const void *restrict b,
                                      void *restrict o, int n)
{{
    for (int i = 0; i + 16 <= n; i += 16) {{
        {r.body}
    }}
}}
"""
            )
            continue
        if r.cls == "x":
            dx, dy, dr, store = (
                '__m128d x = _mm_loadu_pd((const double *)((const char *)a + i));',
                '__m128d y = _mm_loadu_pd((const double *)((const char *)b + i));',
                '__m128d R;',
                '_mm_storeu_pd((double *)((char *)o + i), R);',
            )
        else:
            dx, dy, dr, store = CLASS_DECLS[r.cls]
        out.append(
            f"""\
__attribute__((noinline)) void {r.sym}(const void *restrict a, const void *restrict b,
                                      void *restrict o, int n)
{{
    for (int i = 0; i + 16 <= n; i += 16) {{
        {dx}
        {dy}
        {dr}
        R = {r.body};
        {store}
    }}
}}
"""
        )
    calls = []
    for r in rows:
        calls.append(
            f'    {{ volatile int q; for (q = 0; q < REPS; q++) {r.sym}(DA, DB, DO, NN); }}'
        )
        calls.append(f'    printf("{r.sym} %llx\\n", fnv1a((unsigned char *)DO, sizeof DO));')
    out.append(
        """\
#define NN 512
#define REPS 40
static double DA[NN + 8] __attribute__((aligned(16)));
static double DB[NN + 8] __attribute__((aligned(16)));
static double DO[NN + 8] __attribute__((aligned(16)));

int main(void)
{
    for (int i = 0; i < NN + 8; i++) {
        DA[i] = (double)(long long)((i * 37) % 251) - 125.0;
        DB[i] = (double)(long long)((i * 11) % 97) - 47.0;
    }
"""
        + "\n".join(calls)
        + """
    return 0;
}
"""
    )
    return "".join(out)


# ---------------------------------------------------------------------------
# Compilation, census, execution
# ---------------------------------------------------------------------------


@dataclass
class Toolchain:
    name: str
    cc: str
    version: str


def toolchains() -> list[Toolchain]:
    found: list[Toolchain] = []
    cands = [
        ("lccc", os.environ.get("CCC") or str(REPO / "target/fastbuild/lccc")),
        ("gcc", os.environ.get("GCC", "gcc-16")),
        ("clang", os.environ.get("CLANG", "clang")),
    ]
    for name, cc in cands:
        path = cc if os.path.isabs(cc) else shutil.which(cc)
        if not path or not Path(path).exists():
            continue
        try:
            v = subprocess.run([path, "--version"], capture_output=True, text=True, timeout=60)
        except (OSError, subprocess.SubprocessError):
            continue
        first = (v.stdout or "").splitlines()[:1]
        found.append(Toolchain(name, path, first[0] if first else "?"))
    return found


# An instruction line: any line whose first token starts with a lowercase
# letter.  The leading whitespace is OPTIONAL on purpose -- LCCC emits
# branches and jumps unindented (`jg .LBB3`, `jle .LBB2`) while GCC and Clang
# indent every instruction with a tab.  Requiring indentation therefore
# counted the oracles' branches and silently dropped LCCC's, understating
# every LCCC row by its jump count and flattering the comparison.  A label
# (`.LBB2:`) and a directive (`.p2align`) both start with `.`, so they stay
# excluded.
INSN_RE = re.compile(r"^\s*([a-z][a-z0-9.]*)\b")


def annotate(level: str, message: str) -> None:
    """Surface a diagnostic as a GitHub Actions annotation.

    Hosted CI job logs are not always retrievable (the blob-storage download
    is a separate, sometimes unavailable service), but the check-run
    ANNOTATIONS are part of the API.  A gate that fails without saying why is
    an unusable gate: every failure line is therefore mirrored into an
    annotation, which is readable from `gh api
    repos/<owner>/<repo>/check-runs/<id>/annotations` even when the log is
    not.  Outside Actions the prefix would be noise, so it is suppressed.
    """
    if os.environ.get("GITHUB_ACTIONS") != "true":
        return
    flat = message.replace("\n", " | ")
    print(f"::{level} title=sse2-codegen-matrix::{flat}")


def extract_body(asm: str, sym: str) -> list[str]:
    lines = asm.splitlines()
    start = None
    for i, ln in enumerate(lines):
        if re.match(rf"^{re.escape(sym)}:\s*(#.*)?$", ln):
            start = i + 1
            break
    if start is None:
        return []
    body: list[str] = []
    for ln in lines[start:]:
        if ln.startswith(".size") and sym in ln:
            break
        # A *global* label ends the function; internal labels (.LBB3:,
        # .L2:, # %bb.0:) do not.  Breaking on those measured only the
        # prologue block of LCCC's output and understated every row.
        if ln.startswith(".cfi_endproc"):
            break
        if re.match(r"^[A-Za-z_][\w.$]*:\s*(#.*)?$", ln) and body:
            break
        body.append(ln)
    return body


def census(body: list[str]) -> dict:
    counts: dict[str, int] = {}
    total = 0
    for ln in body:
        s = ln.strip()
        if not s or s.startswith((".", "#", "//", ";")):
            continue
        m = INSN_RE.match(ln)
        if not m:
            continue
        mn = m.group(1)
        if mn.startswith(("cfi_", "lccc_")):
            continue
        counts[mn] = counts.get(mn, 0) + 1
        total += 1
    return {"instructions": total, "mnemonics": dict(sorted(counts.items()))}


def run(cmd, timeout=900):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


def parse_hashes(stdout: str) -> dict[str, str]:
    out = {}
    for ln in stdout.splitlines():
        parts = ln.split()
        if len(parts) == 2:
            out[parts[0]] = parts[1]
    return out


def time_exe(exe: Path, rounds: int) -> dict:
    import statistics
    import time

    times = []
    for _ in range(rounds):
        t0 = time.process_time()
        subprocess.run([str(exe)], capture_output=True, timeout=1800)
        times.append(time.process_time() - t0)
    return {"min": round(min(times), 4), "median": round(statistics.median(times), 4)}


# Mnemonics that carry no evidence about the vector lowering: scalar
# bookkeeping the compiler is *entitled* to shrink away.  The disappearance
# check exists to catch "the SIMD instruction this row is about stopped being
# emitted" (a fidelity regression), not to freeze the prologue.  When a change
# eliminates a frame entirely — `subq $N, %rsp` / `addq $N, %rsp` / the
# `pushq`/`popq` pair around them — those mnemonics vanish and the row gets
# FASTER; pinning them would have rejected a 4-instruction win on 65 rows.
BOOKKEEPING_MNEMONICS = frozenset(
    {
        "addl", "addq", "subl", "subq", "leal", "leaq", "movl", "movq",
        "movslq", "movzbl", "movzwl", "pushq", "popq", "ret", "xorl", "xorq",
        "cmpl", "cmpq", "testl", "testq", "andl", "andq", "orl", "orq",
        "sarl", "sarq", "shrl", "shrq", "shll", "shlq", "salq", "imull",
        "incl", "incl", "decl", "jmp", "nop", "nopw", "nopl", "cltq",
    }
)

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--report", action="store_true")
    ap.add_argument("--update-baseline", action="store_true")
    ap.add_argument("--mode", choices=["static", "timing"], default="static")
    ap.add_argument("--configs", default=",".join(DEFAULT_CONFIGS))
    ap.add_argument("--timing-rounds", type=int, default=5)
    ap.add_argument("--json", default="")
    ap.add_argument("--only", default="")
    ap.add_argument("--strict-expect", action="store_true")
    ap.add_argument(
        "--self-test",
        action="store_true",
        help="perturb the baseline in memory and assert the checker notices "
        "(proves the ratchet still has teeth after a refactor)",
    )
    ap.add_argument(
        "--strict-oracle-debt",
        action="store_true",
        help="also fail when LCCC's gap to the LIVE oracle grows; host-dependent, "
        "so only meaningful with the baseline's own pinned oracle builds",
    )
    args = ap.parse_args()

    configs = [c for c in args.configs.split(",") if c]
    tcs = toolchains()
    names = [t.name for t in tcs]
    if "lccc" not in names:
        print("check_sse2_codegen_matrix: lccc not found (set CCC=)", file=sys.stderr)
        return 2
    for n in ("gcc", "clang"):
        if n not in names:
            print(f"check_sse2_codegen_matrix: oracle {n!r} unavailable -- skipped", file=sys.stderr)
            annotate(
                "warning",
                f"oracle {n} unavailable on this host: its side of every "
                f"differential comparison was skipped",
            )

    rows = [r for r in ROWS if not args.only or args.only in r.mnemonic]
    tmp = Path(tempfile.mkdtemp(prefix="sse2mtx-"))
    src = tmp / "kernels.c"
    src.write_text(kernel_source(rows))

    result: dict = {
        "toolchains": {t.name: t.version for t in tcs},
        "configs": configs,
        "rows": {
            r.mnemonic: {"sym": r.sym, "expect": list(r.expect), "note": r.note, "compilers": {}}
            for r in rows
        },
        "unsupported": {},
    }

    for cfg in configs:
        hashes: dict[str, dict[str, str]] = {}
        for tc in tcs:
            asm = tmp / f"{tc.name}-{cfg}.s"
            p = run([tc.cc, f"-{cfg}", "-S", "-o", str(asm), str(src)])
            if p.returncode != 0:
                result["unsupported"][f"{tc.name}-{cfg}"] = (p.stderr or "").splitlines()[:6]
                # One unsupported intrinsic must not hide the other 120 rows:
                # re-compile row by row and record exactly which rows fail.
                for r in rows:
                    entry = solo_entry(tc, cfg, r, tmp)
                    result["rows"][r.mnemonic]["compilers"].setdefault(tc.name, {})[cfg] = entry
                    if entry.get("status") == "ok":
                        pass
                continue
            text = asm.read_text(errors="replace")
            exe = tmp / f"{tc.name}-{cfg}.bin"
            p = run([tc.cc, f"-{cfg}", "-o", str(exe), str(src)])
            exe_ok = p.returncode == 0 and exe.exists()
            if exe_ok:
                q = run([str(exe)])
                if q.returncode == 0:
                    hashes[tc.name] = parse_hashes(q.stdout)
                else:
                    result["unsupported"][f"{tc.name}-{cfg}-run"] = (q.stderr or "")[-400:]
            for r in rows:
                c = census(extract_body(text, r.sym))
                entry = {
                    "status": "ok" if c["instructions"] else "no-body",
                    **c,
                    "missing_expect": [
                        m for m in r.expect if not any(k.startswith(m) for k in c["mnemonics"])
                    ],
                }
                if args.mode == "timing" and exe_ok:
                    entry["time"] = time_exe(exe, args.timing_rounds)
                result["rows"][r.mnemonic]["compilers"].setdefault(tc.name, {})[cfg] = entry
        for r in rows:
            ref = hashes.get("lccc", {}).get(r.sym)
            for other in ("gcc", "clang"):
                got = hashes.get(other, {}).get(r.sym)
                if ref is None or got is None:
                    continue
                e = result["rows"][r.mnemonic]["compilers"].get(other, {}).get(cfg)
                if e is None:
                    continue
                e["hash_match"] = ref == got
                if ref != got:
                    e["status"] = "hash-mismatch"

    tally: dict[str, int] = {}
    for r in rows:
        for cfg in configs:
            v, _, _ = verdict(result["rows"][r.mnemonic], cfg, names)
            tally[v] = tally.get(v, 0) + 1
    result["summary"] = dict(sorted(tally.items()))

    if args.json:
        Path(args.json).parent.mkdir(parents=True, exist_ok=True)
        Path(args.json).write_text(json.dumps(result, indent=1, sort_keys=True))
    if args.report:
        print_report(result, rows, configs, names)
    # A batch build that fails silently downgrades this gate: the harness
    # re-compiles row by row, which changes the translation unit each row is
    # measured in AND (for an oracle) drops that oracle's half of the
    # differential.  Hosted CI provisions both oracles by contract, so there a
    # degraded matrix is a failure; on a developer machine without clang the
    # single-oracle measurement stays usable, so it is a warning there.
    hosted = os.environ.get("GITHUB_ACTIONS") == "true"
    degraded: list[str] = []
    for key, lines in sorted(result.get("unsupported", {}).items()):
        detail = " / ".join(x.strip() for x in lines if x.strip())[:400]
        print(f"check_sse2_codegen_matrix: {key}: batch build unavailable: {detail}")
        annotate("error", f"{key}: batch build unavailable: {detail}")
        if key.startswith("lccc") or hosted:
            degraded.append(f"{key}: batch build unavailable: {detail[:200]}")
        else:
            annotate("warning", f"{key}: measurements are row-at-a-time only")
    if args.update_baseline:
        BASELINE.write_text(
            json.dumps(build_baseline(result, rows, configs, names), indent=1, sort_keys=True) + "\n"
        )
        print(f"wrote {BASELINE}")
    print(
        "matrix: {} rows, config(s) {}, toolchains {}".format(
            len(rows),
            ",".join(configs),
            ", ".join(f"{t.name}={t.version.split()[0]}" for t in tcs),
        )
    )
    if args.check or args.report:
        recorded = json.loads(BASELINE.read_text()).get("toolchains", {})
        for name in sorted(result["toolchains"]):
            now = result["toolchains"][name]
            was = recorded.get(name)
            if was and was != now:
                print(
                    f"note: {name} oracle differs from the baseline's recorded build:\n"
                    f"  baseline: {was}\n"
                    f"  host:     {now}\n"
                    f"  (instruction-count debt and verdicts are comparisons against "
                    f"whatever oracle is installed; the enforced ratchet is LCCC's "
                    f"own count, which is host-independent)",
                    file=sys.stderr,
                )
    if args.self_test:
        # A gate nobody has seen fail is a gate nobody knows works. Tighten one
        # recorded count by one instruction and feed the checker a result that
        # exceeds it: if the checker does not notice, the ratchet is broken.
        #
        # The fixture is SYNTHETIC on purpose. Driving the probe from the live
        # measurement would make the self-test pass or fail with the host: if
        # this compiler ever emitted one instruction fewer than the recorded
        # count, tightening the baseline by one would leave nothing to catch
        # and the self-test would redden CI for a *better* compiler. Here the
        # row's measured count is constructed to be exactly one over the
        # tightened baseline, so the assertion is about the checker.
        probe = json.loads(json.dumps(json.loads(BASELINE.read_text())))
        row = next(iter(sorted(probe.get("debt", {}))))
        cfg = next(iter(probe["debt"][row]))
        recorded = probe["debt"][row][cfg]["lccc"]
        probe["debt"][row][cfg]["lccc"] = recorded - 1
        fake = {
            "rows": {
                row: {
                    "compilers": {
                        "lccc": {cfg: {"status": "ok", "instructions": recorded,
                                       "mnemonics": [], "missing_expect": []}},
                        "gcc": {cfg: {"status": "ok", "instructions": recorded}},
                    }
                }
            }
        }
        with contextlib.redirect_stderr(io.StringIO()), contextlib.redirect_stdout(
            io.StringIO()
        ):
            probe_rows = [r for r in rows if r.mnemonic == row] or rows[:1]
            caught = check_baseline(
                fake, probe_rows, configs, names + ["gcc"], False, False, base=probe
            )
        if caught == 0:
            print(
                "check_sse2_codegen_matrix: SELF-TEST FAILED -- a tightened "
                "baseline was not detected",
                file=sys.stderr,
            )
            return 1
        print(f"check_sse2_codegen_matrix: self-test ok ({row} tightened by 1 is caught)")

    rc = (
        check_baseline(
            result, rows, configs, names, args.strict_expect, args.strict_oracle_debt
        )
        if args.check
        else 0
    )
    if degraded:
        for d in degraded:
            print(f"FAIL degraded matrix: {d}")
            annotate("error", f"degraded matrix: {d}")
        print(
            f"check_sse2_codegen_matrix: {len(degraded)} degraded batch build(s); "
            "the committed baseline cannot be verified against them"
        )
        rc = rc or 1
    shutil.rmtree(tmp, ignore_errors=True)
    return rc


def solo_entry(tc: "Toolchain", cfg: str, r: Row, tmp: Path) -> dict:
    """Census + hash for one row in its own translation unit."""
    d = tmp / "solo"
    d.mkdir(exist_ok=True)
    src = d / f"{r.sym}-{tc.name}-{cfg}.c"
    src.write_text(kernel_source([r], solo=True))
    asm = src.with_suffix(".s")
    p = run([tc.cc, f"-{cfg}", "-S", "-o", str(asm), str(src)])
    if p.returncode != 0:
        return {"status": "compile-error", "detail": (p.stderr or "").splitlines()[:2]}
    body = extract_body(asm.read_text(errors="replace"), r.sym)
    c = census(body)
    entry = {
        "status": "ok" if c["instructions"] else "no-body",
        **c,
        "missing_expect": [
            m for m in r.expect if not any(k.startswith(m) for k in c["mnemonics"])
        ],
    }
    exe = src.with_suffix(".bin")
    p = run([tc.cc, f"-{cfg}", "-o", str(exe), str(src)])
    if p.returncode == 0:
        q = run([str(exe)])
        if q.returncode == 0:
            entry["hash"] = parse_hashes(q.stdout).get(r.sym)
    return entry


def best_oracle(entry: dict, cfg: str, names) -> tuple[str, int] | None:
    best = None
    for n in names:
        if n == "lccc":
            continue
        e = entry["compilers"].get(n, {}).get(cfg)
        if not e or e.get("status") != "ok":
            continue
        if best is None or e["instructions"] < best[1]:
            best = (n, e["instructions"])
    return best


def verdict(entry: dict, cfg: str, names) -> tuple[str, int, int | None]:
    mine = entry["compilers"].get("lccc", {}).get(cfg, {})
    if mine.get("status") != "ok":
        return "blocked", -1, None
    best = best_oracle(entry, cfg, names)
    if best is None:
        return "no-oracle", mine.get("instructions", 0), None
    if mine["instructions"] < best[1]:
        return "win", mine["instructions"], best[1]
    if mine["instructions"] == best[1]:
        return "tie", mine["instructions"], best[1]
    return "loss", mine["instructions"], best[1]


def print_report(result, rows, configs, names):
    print("toolchains:")
    for k, v in result["toolchains"].items():
        print(f"  {k:<6} {v}")
    if result["unsupported"]:
        print("unavailable:")
        for k, v in result["unsupported"].items():
            print(f"  {k}: {v[:2]}")
    print()
    for cfg in configs:
        print(f"=== -{cfg}  (lccc / gcc / clang instruction counts, verdict)")
        hdr = f"{'instruction':<14}{'class':<8}{'lccc':>5}{'gcc':>5}{'clang':>5}{'delta':>7}  verdict"
        print(hdr)
        print("-" * len(hdr))
        for r in rows:
            e = result["rows"][r.mnemonic]
            mine = e["compilers"].get("lccc", {}).get(cfg, {})
            g = e["compilers"].get("gcc", {}).get(cfg, {}).get("instructions")
            c = e["compilers"].get("clang", {}).get(cfg, {}).get("instructions")
            v, mn, best = verdict(e, cfg, names)
            mark = {"win": "WIN", "tie": "tie", "loss": "LOSS", "blocked": "BLOCKED"}.get(v, v)
            delta = f"{mn - best:+d}" if best is not None else "-"
            print(
                f"{r.mnemonic:<14}{r.cls:<8}"
                f"{mine.get('instructions', '-'):>5}{str(g or '-'):>5}{str(c or '-'):>5}"
                f"{delta:>7}  {mark}"
            )
        print()
    print("summary:", result["summary"])


def build_baseline(result, rows, configs, names):
    base = {"toolchains": result["toolchains"], "configs": configs, "verdicts": {},
            "debt": {}, "mnemonics": {}}
    for r in rows:
        e = result["rows"][r.mnemonic]
        for cfg in configs:
            v, mn, best = verdict(e, cfg, names)
            base["verdicts"].setdefault(r.mnemonic, {})[cfg] = v
            if v == "loss":
                base["debt"].setdefault(r.mnemonic, {})[cfg] = {"lccc": mn, "best_oracle": best}
            mine = e["compilers"].get("lccc", {}).get(cfg, {})
            if mine.get("status") == "ok":
                base["mnemonics"].setdefault(r.mnemonic, {})[cfg] = list(mine["mnemonics"])
    return base


def check_baseline(
    result,
    rows,
    configs,
    names,
    want_expect: bool = False,
    strict_oracle_debt: bool = False,
    base=None,
) -> int:
    if base is None:
        if not BASELINE.exists():
            print(f"check_sse2_codegen_matrix: missing baseline {BASELINE}", file=sys.stderr)
            return 2
        base = json.loads(BASELINE.read_text())
    failures: list[str] = []
    rank = {"win": 0, "tie": 1, "loss": 2}
    for r in rows:
        e = result["rows"][r.mnemonic]
        for cfg in configs:
            v, mn, best = verdict(e, cfg, names)
            if v in ("blocked", "no-oracle"):
                continue
            want = base["verdicts"].get(r.mnemonic, {}).get(cfg)
            # The VERDICT is a comparison against the live oracle, so it is
            # host-dependent in the same way the debt is: an oracle built from
            # a different 16.2.0 snapshot can shave a row by one instruction
            # and turn a recorded `tie` into a `loss` without LCCC changing at
            # all.  The count ratchet above is the host-independent invariant;
            # the verdict ranking is enforced only in `--strict-oracle-debt`
            # (pinned-oracle hosts), and always reported.
            if (
                strict_oracle_debt
                and want is not None
                and rank.get(v, 3) > rank.get(want, 2)
            ):
                failures.append(f"{r.mnemonic} [{cfg}]: {want} -> {v} (lccc {mn}, best {best})")
            debt = base["debt"].get(r.mnemonic, {}).get(cfg)
            if debt:
                # RATCHET ON OUR OWN OUTPUT. The checked invariant is LCCC's
                # instruction count, which this repository fully controls and
                # which is identical on every host.  The *oracle* count is not:
                # GCC 16.2.0 from a distribution and GCC 16.2.0 from an
                # upstream tarball are different builds, and a one-instruction
                # difference in the oracle would fail an innocent change once
                # LCCC is close to parity (which is now the normal case: the
                # median row is +2 instructions, not +20).  Comparing our count
                # to our own recorded count is therefore strictly the stronger
                # statement -- a regression in LCCC fails on any host, and a
                # different oracle build cannot.
                if mn > debt["lccc"]:
                    failures.append(
                        f"{r.mnemonic} [{cfg}]: instruction count grew "
                        f"{debt['lccc']} -> {mn}"
                    )
                # The oracle gap is still checked when the caller asks for it
                # (`--strict-oracle-debt`), which is meaningful only on a host
                # whose oracles match the pinned toolchains recorded in the
                # baseline.
                if strict_oracle_debt and best is not None:
                    before = debt["lccc"] - debt["best_oracle"]
                    after = mn - best
                    if after > before:
                        failures.append(
                            f"{r.mnemonic} [{cfg}]: debt grew {before} -> {after} instructions"
                        )
            # Lowering fidelity is checked against the recorded baseline, not
            # against the table's ideal: a mnemonic the backend does not emit
            # today is a recorded finding (`missing_expect` in the report), and
            # the gate's job is to prove it never *stops* being emitted.  That
            # keeps the gate monotone; `--strict-expect` (local/evidence use)
            # enforces the ideal set instead.
            mine = e["compilers"].get("lccc", {}).get(cfg, {})
            if want_expect and mine.get("status") == "ok" and mine.get("missing_expect"):
                failures.append(
                    f"{r.mnemonic} [{cfg}]: expected mnemonic(s) {mine['missing_expect']} absent"
                )
            if mine.get("status") == "ok":
                recorded = set(base.get("mnemonics", {}).get(r.mnemonic, {}).get(cfg, []))
                now = set(mine.get("mnemonics", {}))
                lost = sorted(m for m in recorded - now if m not in BOOKKEEPING_MNEMONICS)
                if lost:
                    failures.append(
                        f"{r.mnemonic} [{cfg}]: mnemonic(s) {lost} no longer emitted"
                    )
            for other in ("gcc", "clang"):
                o = e["compilers"].get(other, {}).get(cfg, {})
                if o.get("hash_match") is False:
                    failures.append(f"{r.mnemonic} [{cfg}]: output differs from {other}")
    for f in failures:
        print(f"FAIL {f}")
        annotate("error", f)
    if failures:
        print(f"check_sse2_codegen_matrix: {len(failures)} failure(s)")
        return 1
    print("check_sse2_codegen_matrix: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
