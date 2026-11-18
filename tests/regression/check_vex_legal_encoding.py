#!/usr/bin/env python3
"""VEX encoding parity for 128-bit and scalar SIMD code under -mavx.

GCC 16.2 encodes every 128-bit and scalar SSE operation in VEX form once AVX
is enabled.  LCCC used to emit legacy SSE (`addps`, `movss`, `pshufd`, ...)
for those operations under `-mavx`, and the same legacy moves under
`-mavx2`.  A legacy write after dirty upper YMM state pays an SSE/AVX
transition cost on Intel cores, so this is a runtime defect, not only a
style one.

The contract pinned here, for a fixed probe of 128-bit and scalar operations:

  * `-O2 -mavx` and `-O2 -mavx2`: the only legacy SSE instructions are the
    two in `s_sqrtsd`.  That function tail-calls `sqrt@PLT`, which may observe
    every upper register, so its upper-preserving encoding is kept (the
    `vex_promote` soundness rule 2).  Every other function is fully VEX.
  * `-O2` (SSE2 only): no VEX instruction and no `LCCC_VEX_LEGAL` marker
    anywhere, so VEX cannot reach a target without AVX.

A negative self-test feeds a legacy listing and requires the counter to flag it.

Usage:
    check_vex_legal_encoding.py --lccc target/fastbuild/lccc [--gcc gcc-16]
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

PROBE = r"""
#include <immintrin.h>
__m128 p_addps(__m128 a, __m128 b) { return _mm_add_ps(a, b); }
__m128 p_mulps(__m128 a, __m128 b) { return _mm_mul_ps(a, b); }
__m128 p_shufps(__m128 a, __m128 b) { return _mm_shuffle_ps(a, b, 0x1b); }
__m128 p_movhlps(__m128 a, __m128 b) { return _mm_movehl_ps(a, b); }
__m128 p_addss(__m128 a, __m128 b) { return _mm_add_ss(a, b); }
__m128 p_movss(__m128 a, __m128 b) { return _mm_move_ss(a, b); }
__m128 p_sqrtps(__m128 a) { return _mm_sqrt_ps(a); }
__m128 p_rcpps(__m128 a) { return _mm_rcp_ps(a); }
__m128d p_addpd(__m128d a, __m128d b) { return _mm_add_pd(a, b); }
__m128d p_unpckhpd(__m128d a, __m128d b) { return _mm_unpackhi_pd(a, b); }
__m128d p_movedup(__m128d a) { return _mm_movedup_pd(a); }
__m128i p_addepi32(__m128i a, __m128i b) { return _mm_add_epi32(a, b); }
__m128i p_pand(__m128i a, __m128i b) { return _mm_and_si128(a, b); }
__m128i p_unpcklbw(__m128i a, __m128i b) { return _mm_unpacklo_epi8(a, b); }
__m128i p_pshufd(__m128i a) { return _mm_shuffle_epi32(a, 0x1b); }
__m128i p_slli(__m128i a) { return _mm_slli_epi32(a, 3); }
__m128i p_srai(__m128i a) { return _mm_srai_epi16(a, 3); }
__m128i p_packs(__m128i a, __m128i b) { return _mm_packs_epi32(a, b); }
float s_addss(float a, float b) { return a + b; }
double s_addsd(double a, double b) { return a + b; }
float s_cvtsi(int a) { return (float)a; }
int s_cvtt(float a) { return (int)a; }
int s_cmpss(float a, float b) { return a < b; }
double s_sqrtsd(double a) { return __builtin_sqrt(a); }
"""

SSE2_PROBE = r"""
#include <emmintrin.h>
__m128 q_addps(__m128 a, __m128 b) { return _mm_add_ps(a, b); }
__m128i q_unpack(__m128i a, __m128i b) { return _mm_unpacklo_epi8(a, b); }
__m128d q_mul(__m128d a, __m128d b) { return _mm_mul_pd(a, b); }
float q_scalar(float a, float b) { return a + b; }
"""

# Legacy SSE mnemonics that have a VEX form.  `xmm` must appear in the
# operands, so GPR `addq`/`subq` are excluded.
LEGACY = re.compile(
    r"^(add|sub|mul|div|min|max|sqrt|rcp|rsqrt|and|andn|or|xor|cmp|"
    r"unpckl|unpckh|shuf|movhl|movlh|movl|movh|movd|movq|movdq[au]|movap[sd]|"
    r"movup[sd]|movs[sd]|movsh|movhp[sd]|movlp[sd]|movmsk|cvt|pshuf|punpck|"
    r"padd|psub|pmul|pmadd|pand|pandn|por|pxor|pcmp|pmax|pmin|pavg|psad|"
    r"psll|psrl|psra|pack|pslldq|psrldq|palignr|pshufb|movddup|movsldup|"
    r"movshdup|pmov|blend|ptest|pinsr|pextr|extractps|insertps|"
    r"ucomis[sd]|comis[sd])"
    r"(ps|pd|ss|sd|dqa|dqu|dq|i|d|q|w|b|epi\w*|epu\w*|si128|ldq|s|\b|$)"
)
# The only legacy instructions the pinned contract accepts, per function.
ALLOWED_LEGACY = {"s_sqrtsd": {"movapd", "ucomisd"}}


def scan(text: str) -> dict[str, dict[str, int]]:
    """Map function -> legacy mnemonic -> count, over an assembly listing."""
    out: dict[str, dict[str, int]] = {}
    fn = "<top>"
    for raw in text.splitlines():
        s = raw.strip()
        if not s:
            continue
        if s.endswith(":") and not s.startswith("."):
            fn = s[:-1]
            continue
        if s.startswith("#") or s.startswith("."):
            continue
        mn = s.split()[0]
        if mn.startswith("v") or "xmm" not in s:
            continue
        if LEGACY.match(mn):
            per = out.setdefault(fn, {})
            per[mn] = per.get(mn, 0) + 1
    return out


def compile_listing(compiler: str, flags: list[str], src: str, tmp: Path) -> str:
    c = tmp / "probe.c"
    c.write_text(src)
    proc = subprocess.run(
        [compiler, *flags, "-S", "-o", "-", str(c)],
        capture_output=True,
        text=True,
        timeout=120,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"{compiler} {' '.join(flags)} failed: {proc.stderr[:400]}")
    return proc.stdout


def check_avx(listing: str, label: str) -> list[str]:
    errors = []
    legacy = scan(listing)
    for fn, mns in legacy.items():
        allowed = ALLOWED_LEGACY.get(fn, set())
        extra = set(mns) - allowed
        if extra:
            errors.append(f"{label}: {fn} keeps legacy {sorted(extra)}")
    for fn in ALLOWED_LEGACY:
        if fn not in legacy:
            errors.append(f"{label}: expected legacy pins in {fn} not seen")
    if "LCCC_VEX_LEGAL 1" not in listing:
        errors.append(f"{label}: no LCCC_VEX_LEGAL marker emitted under AVX")
    if not re.search(r"^\s+v[a-z]", listing, re.M):
        errors.append(f"{label}: no VEX instruction emitted under AVX")
    return errors


def check_sse2(listing: str, label: str) -> list[str]:
    errors = []
    if re.search(r"^\s+v[a-z]", listing, re.M):
        errors.append(f"{label}: VEX instruction emitted without AVX")
    if "LCCC_VEX_LEGAL" in listing:
        errors.append(f"{label}: VEX-legal marker emitted without AVX")
    return errors


def self_test() -> list[str]:
    legacy_listing = "f:\n    addps %xmm1, %xmm0\n    ret\n"
    if not scan(legacy_listing):
        return ["self-test: a legacy addps listing was not flagged"]
    vex_listing = "f:\n    vaddps %xmm1, %xmm0, %xmm0\n    ret\n"
    if scan(vex_listing):
        return ["self-test: a VEX listing was flagged as legacy"]
    gpr = "f:\n    addq $1, %rax\n    ret\n"
    if scan(gpr):
        return ["self-test: GPR addq was flagged as legacy SIMD"]
    return []


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--lccc", required=True, help="path to the lccc driver")
    ap.add_argument("--gcc", default=None, help="optional GCC for a parity cross-check")
    args = ap.parse_args()

    errors = self_test()
    with tempfile.TemporaryDirectory() as td:
        tmp = Path(td)
        for flags in (["-O2", "-mavx"], ["-O2", "-mavx2"]):
            listing = compile_listing(args.lccc, flags, PROBE, tmp)
            errors += check_avx(listing, f"lccc {' '.join(flags)}")
        sse2 = compile_listing(args.lccc, ["-O2"], SSE2_PROBE, tmp)
        errors += check_sse2(sse2, "lccc -O2 (SSE2)")
        if args.gcc:
            gl = compile_listing(args.gcc, ["-O2", "-mavx"], PROBE, tmp)
            gerr = scan(gl)
            if gerr:
                errors.append(f"gcc -O2 -mavx unexpectedly emits legacy SSE: {gerr}")
    for e in errors:
        print("FAIL", e)
    if errors:
        print("check_vex_legal_encoding: FAILED")
        return 1
    print("check_vex_legal_encoding: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
