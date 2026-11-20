#!/usr/bin/env python3
"""Accept/reject verdicts for AVX1 and AVX2 intrinsics under each -m flag.

Each case pins the verdict that GCC 16.2 produces and the verdict LCCC must
produce. The check fails if either compiler disagrees with the pin, so an ISA
gate cannot silently drift in either direction:

  * an AVX2-only name (`_mm256_cmpgt_epi8`, `_mm256_add_epi8`, ...) must be
    REJECTED under -mavx (GCC: "target specific option mismatch") and ACCEPTED
    under -mavx2;
  * an AVX1 name must stay ACCEPTED under -mavx (a gate that is too strict
    breaks every AVX1 program);
  * a name must not be accepted without any -m flag when GCC rejects it.

Every case is compiled in its own translation unit and the result is stored to
a volatile sink, so the call is live and cannot be discarded before the check.
A negative self-test pins a verdict both compilers reject and requires the
harness to report it, so the gate can demonstrably fail.

Usage:
    check_avx_isa_verdicts.py --lccc target/fastbuild/lccc [--gcc gcc-16]
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import os
import subprocess
import sys
import tempfile
from pathlib import Path

NOHDR_PRELUDE = """\
typedef float v8sf __attribute__((vector_size(32)));
typedef double v4df __attribute__((vector_size(32)));
typedef long long v4di __attribute__((vector_size(32)));
volatile long long sink;
void f(v8sf x, v4df d, v4di a, v4di b) {
"""

PRELUDE = """\
#include <immintrin.h>
volatile long long sink;
void f(__m256i a, __m256i b, __m256i c, __m256 x, __m128i p, __m128i q) {
"""

BODY = """\
    __typeof__(%s) r = %s;
    sink = ((volatile long long *)&r)[0];
}
"""

ACC, REJ = "accept", "reject"

# AVX2-only 256-bit integer operations (two operands). GCC declares each under
# `target("avx2")`, so -mavx rejects every one of them.
AVX2_BINARY = [
    "_mm256_cmpgt_epi8", "_mm256_cmpgt_epi16", "_mm256_cmpgt_epi32",
    "_mm256_cmpeq_epi8", "_mm256_cmpeq_epi16", "_mm256_cmpeq_epi32",
    "_mm256_add_epi8", "_mm256_add_epi16", "_mm256_sub_epi32",
    "_mm256_adds_epi16", "_mm256_subs_epu8", "_mm256_avg_epu8",
    "_mm256_sign_epi8", "_mm256_mullo_epi16", "_mm256_madd_epi16",
    "_mm256_max_epi16", "_mm256_min_epu16", "_mm256_unpacklo_epi8",
    "_mm256_packs_epi16", "_mm256_shuffle_epi8", "_mm256_sllv_epi32",
    "_mm256_srav_epi32", "_mm256_sad_epu8", "_mm256_permutevar8x32_epi32",
    # 256-bit integer logic is AVX2 (VPAND/VPOR/VPXOR/VPANDN ymm); AVX1 only
    # has the float forms. GCC: "target specific option mismatch" under -mavx.
    "_mm256_and_si256", "_mm256_or_si256", "_mm256_xor_si256",
    "_mm256_andnot_si256",
]

# AVX1 operations that must remain accepted under -mavx. Each row is spelled
# out explicitly so every argument has the type its intrinsic declares: a
# type error would be a different verdict and must not masquerade as an ISA one.
AVX1_CASES: list[tuple[str, str, str]] = [
    ("_mm256_add_ps(x, x)", "-O2 -mavx", ACC),
    ("_mm256_addsub_ps(x, x)", "-O2 -mavx", ACC),
    ("_mm256_permutevar_ps(x, a)", "-O2 -mavx", ACC),
    ("_mm256_castsi256_ps(a)", "-O2 -mavx", ACC),
]

CASES: list[tuple[str, str, str]] = []
for name in AVX2_BINARY:
    CASES.append((f"{name}(a, b)", "-O2 -mavx", REJ))
    CASES.append((f"{name}(a, b)", "-O2 -mavx2", ACC))
CASES += AVX1_CASES
# With no -m flag, GCC rejects every 256-bit intrinsic, AVX1 ones included.
CASES.append(("_mm256_add_ps(x, x)", "-O2", REJ))
CASES.append(("_mm256_add_epi8(a, b)", "-O2", REJ))
# A wrong argument count under -mavx2 is a rejection by arity, not by ISA; this
# pins that the arity path still reports (rather than silently accepting).
CASES.append(("_mm256_add_epi8(a)", "-O2 -mavx2", REJ))

# Without <immintrin.h> the public names are not declared at all (GCC: implicit
# declaration error). LCCC once bound them through its frontend table, which
# silently resolved `_mm256_andnot_ps` to the AVX2 VPANDN under -mavx.
NOHDR_CASES: list[tuple[str, str, str]] = [
    ("_mm256_andnot_ps(x, x)", "-O2 -mavx", REJ),
    ("_mm256_andnot_pd(d, d)", "-O2 -mavx", REJ),
    ("_mm256_andnot_si256(a, b)", "-O2 -mavx2", REJ),
]

# Negative control: this verdict is rejected by BOTH compilers but pinned as
# accept. The harness must report a mismatch, otherwise the gate can never fail.
SELF_TEST_CASE = ("_mm256_cmpgt_epi8(a, b)", "-O2 -mavx", ACC)


def verdict(cc: str, flags: str, src: Path, out_dir: Path) -> tuple[int, str]:
    target = out_dir / (src.stem + ".o")
    cmd = [cc, *flags.split(), "-c", str(src), "-o", str(target)]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return 124, "timeout"
    lines = (proc.stderr or "").strip().splitlines()
    return proc.returncode, lines[-1][:160] if lines else ""


def run_case(job: tuple[int, str, str, str, str, str, str, Path]) -> dict:
    idx, expr, flags, pinned, prelude, lccc, gcc, tmp = job
    src = tmp / f"verdict_{idx}.c"
    src.write_text(prelude + BODY % (expr, expr))
    lc, lerr = verdict(lccc, flags, src, tmp)
    gc, gerr = verdict(gcc, flags, src, tmp)
    return {
        "expr": expr, "flags": flags, "pinned": pinned,
        "lccc": ACC if lc == 0 else REJ, "gcc": ACC if gc == 0 else REJ,
        "lerr": lerr, "gerr": gerr,
    }


def self_test(lccc: str, gcc: str, tmp: Path) -> None:
    expr, flags, pinned = SELF_TEST_CASE
    probe = run_case((-1, expr, flags, pinned, PRELUDE, lccc, gcc, tmp))
    if probe["lccc"] == ACC and probe["gcc"] == ACC:
        raise SystemExit("self-test: harness accepted a verdict both compilers reject")
    if probe["lccc"] == pinned and probe["gcc"] == pinned:
        raise SystemExit("self-test: a deliberately wrong pin was not reported as a mismatch")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--lccc", type=Path, required=True)
    ap.add_argument("--gcc", default=os.environ.get("GCC_BIN", "gcc-16"))
    args = ap.parse_args()

    with tempfile.TemporaryDirectory(prefix="avx-isa-verdicts-") as tmp_s:
        tmp = Path(tmp_s)
        self_test(str(args.lccc), args.gcc, tmp)
        jobs = [(i, e, f, p, PRELUDE, str(args.lccc), args.gcc, tmp)
                for i, (e, f, p) in enumerate(CASES)]
        jobs += [(len(CASES) + i, e, f, p, NOHDR_PRELUDE, str(args.lccc), args.gcc, tmp)
                 for i, (e, f, p) in enumerate(NOHDR_CASES)]
        with cf.ThreadPoolExecutor(max_workers=max(1, os.cpu_count() or 2)) as pool:
            results = list(pool.map(run_case, jobs))

    bad = [r for r in results if r["lccc"] != r["pinned"] or r["gcc"] != r["pinned"]]
    for r in bad:
        print(f"MISMATCH {r['flags']!r} pinned={r['pinned']} lccc={r['lccc']} "
              f"gcc={r['gcc']}\n  {r['expr']}\n  lccc: {r['lerr']}\n  gcc:  {r['gerr']}")
    accepted = sum(1 for r in results if r["pinned"] == ACC)
    print(f"cases={len(results)} accept={accepted} reject={len(results) - accepted} "
          f"mismatches={len(bad)}")
    if bad:
        print("check_avx_isa_verdicts: FAILED")
        return 1
    print("check_avx_isa_verdicts: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
