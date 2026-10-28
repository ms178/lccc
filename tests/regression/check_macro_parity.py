#!/usr/bin/env python3
"""GP-23: macro-parity budget ratchet against GCC 16.2.

Compares complete `-std=gnu23 -dM -E` dumps (empty TU, x86-64) between
GCC 16.2.0 and LCCC and fails on ANY divergence outside three pinned,
individually-justified budgets:

* ``GCC_ONLY``    - names GCC predefines and LCCC does not. Every entry is
                    a deliberate, documented skip from the 2026-10-28 GP-05
                    sweep (decimal-FP macros LCCC does not implement, asm/CFI
                    capability claims that are not verified, HLE hints, the
                    C++ ABI identity).
* ``LCCC_ONLY``   - names LCCC predefines and GCC does not: keyword-vs-macro
                    mechanisms GCC spells differently (`__extension__`,
                    `offsetof`, `__int8` ...), `-dM`-listing quirks
                    (`__FILE__`/`__DATE__` are expanded by GCC but not
                    listed), the documented PIC/PIE and C23 decimal/width
                    claims, and `_FloatN` spellings.
* ``VALUE_DIFFS`` - common names whose replacement bodies differ (GCC's
                    cast/suffix spellings versus LCCC's literals; the
                    pinned set is measured-stable).

A pinned entry that disappears (direction improved) is reported as a STALE
pin and fails too, so the budgets can only move with a reviewed edit of
this file. New divergence in either direction fails immediately.
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

GCC_ONLY = (
    '__ATOMIC_HLE_ACQUIRE',
    '__ATOMIC_HLE_RELEASE',
    '__DEC64X_EPSILON__',
    '__DEC64X_MANT_DIG__',
    '__DEC64X_MAX_EXP__',
    '__DEC64X_MAX__',
    '__DEC64X_MIN_EXP__',
    '__DEC64X_MIN__',
    '__DEC64X_SUBNORMAL_MIN__',
    '__GCC_ASM_FLAG_OUTPUTS__',
    '__GCC_CONSTRUCTIVE_SIZE',
    '__GCC_DESTRUCTIVE_SIZE',
    '__HAVE_SPECULATION_SAFE_VALUE',
    '__code_model_small__',
)

LCCC_ONLY = (
    '_Float32',
    '_Float32x',
    '_Float64',
    '_Float64x',
    '__BASE_FILE__',
    '__CHAR_WIDTH__',
    '__DATE__',
    '__DEC128_DIG__',
    '__DEC32_DIG__',
    '__DEC64_DIG__',
    '__FILE__',
    '__FLT128_TRUE_MIN__',
    '__GNUC_VA_LIST',
    '__LEAF',
    '__LEAF_ATTR',
    '__PIC__',
    '__PIE__',
    '__SIZEOF_DECIMAL128__',
    '__SIZEOF_DECIMAL32__',
    '__SIZEOF_DECIMAL64__',
    '__STDC_DEC_FP__',
    '__STDC_IEC_60559_TYPES_EXT__',
    '__TIME__',
    '__builtin_offsetof(type,member)',
    '__extension__',
    '__int16',
    '__int32',
    '__int64',
    '__int8',
    '__pic__',
    '__pie__',
    '__wur',
)

VALUE_DIFFS = (
    ('__BYTE_ORDER__',
     '__ORDER_LITTLE_ENDIAN__',
     '1234'),
    ('__DBL_DENORM_MIN__',
     '((double)4.94065645841246544176568792868221372e-324L)',
     '4.94065645841246544176568792868221372e-324'),
    ('__DBL_EPSILON__',
     '((double)2.22044604925031308084726333618164062e-16L)',
     '2.22044604925031308084726333618164062e-16'),
    ('__DBL_MAX__',
     '((double)1.79769313486231570814527423731704357e+308L)',
     '1.79769313486231570814527423731704357e+308'),
    ('__DBL_MIN__',
     '((double)2.22507385850720138309023271733240406e-308L)',
     '2.22507385850720138309023271733240406e-308'),
    ('__DEC128_EPSILON__',
     '1E-33DL',
     '1e-33DL'),
    ('__DEC128_MAX__',
     '9.999999999999999999999999999999999E6144DL',
     '9.999999999999999999999999999999999e6144DL'),
    ('__DEC128_MIN__',
     '1E-6143DL',
     '1e-6143DL'),
    ('__DEC128_SUBNORMAL_MIN__',
     '0.000000000000000000000000000000001E-6143DL',
     '0.000000000000000000000000000000001e-6143DL'),
    ('__DEC32_EPSILON__',
     '1E-6DF',
     '1e-6DF'),
    ('__DEC32_MAX__',
     '9.999999E96DF',
     '9.999999e96DF'),
    ('__DEC32_MIN__',
     '1E-95DF',
     '1e-95DF'),
    ('__DEC32_SUBNORMAL_MIN__',
     '0.000001E-95DF',
     '0.000001e-95DF'),
    ('__DEC64_EPSILON__',
     '1E-15DD',
     '1e-15DD'),
    ('__DEC64_MAX__',
     '9.999999999999999E384DD',
     '9.999999999999999e384DD'),
    ('__DEC64_MIN__',
     '1E-383DD',
     '1e-383DD'),
    ('__DEC64_SUBNORMAL_MIN__',
     '0.000000000000001E-383DD',
     '0.000000000000001e-383DD'),
    ('__INT_MAX__',
     '0x7fffffff',
     '2147483647'),
    ('__LONG_LONG_MAX__',
     '0x7fffffffffffffffLL',
     '9223372036854775807LL'),
    ('__LONG_MAX__',
     '0x7fffffffffffffffL',
     '9223372036854775807L'),
    ('__PTRDIFF_MAX__',
     '0x7fffffffffffffffL',
     '9223372036854775807L'),
    ('__SCHAR_MAX__',
     '0x7f',
     '127'),
    ('__SHRT_MAX__',
     '0x7fff',
     '32767'),
    ('__SIG_ATOMIC_MAX__',
     '0x7fffffff',
     '2147483647'),
    ('__SIG_ATOMIC_MIN__',
     '(-__SIG_ATOMIC_MAX__ - 1)',
     '(-2147483647-1)'),
    ('__SIZE_MAX__',
     '0xffffffffffffffffUL',
     '18446744073709551615UL'),
    ('__WCHAR_MAX__',
     '0x7fffffff',
     '2147483647'),
    ('__WCHAR_MIN__',
     '(-__WCHAR_MAX__ - 1)',
     '(-2147483647-1)'),
    ('__WINT_MAX__',
     '0xffffffffU',
     '4294967295U'),
)


def dump(compiler: list[str]) -> dict[str, str]:
    proc = subprocess.run(
        [*compiler, "-std=gnu23", "-dM", "-E", "-x", "c", "/dev/null"],
        cwd=ROOT, text=True, capture_output=True, timeout=30,
    )
    if proc.returncode:
        raise RuntimeError(f"{compiler} -dM failed: {proc.stderr[:200]}")
    out = {}
    for line in proc.stdout.splitlines():
        m = re.match(r"#define (\S+)(?:\s+(.*))?$", line)
        if m:
            out[m.group(1)] = m.group(2) or ""
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default="gcc-16")
    args = parser.parse_args()
    g = dump([args.gcc])
    l = dump([args.lccc])
    failures: list[str] = []

    live_gcc_only = sorted(set(g) - set(l))
    live_lccc_only = sorted(set(l) - set(g))
    for name in live_gcc_only:
        if name not in GCC_ONLY:
            failures.append(f"new GCC-only macro: {name} (add to LCCC or pin deliberately)")
    for name in GCC_ONLY:
        if name not in live_gcc_only:
            failures.append(
                f"stale GCC_ONLY pin ({name} is no longer a GCC-only gap): update budgets"
            )
    for name in live_lccc_only:
        if name not in LCCC_ONLY:
            failures.append(f"new LCCC-only macro: {name} (GCC would not predefine it)")
    for name in LCCC_ONLY:
        if name not in live_lccc_only:
            failures.append(f"stale LCCC_ONLY pin ({name} is no longer LCCC-only): update budgets")

    live_diffs = sorted((n, g[n], l[n]) for n in set(g) & set(l) if g[n] != l[n])
    pinned = {n: (gl, ll) for n, gl, ll in VALUE_DIFFS}
    live_map = {n: (gl, ll) for n, gl, ll in live_diffs}
    for n, gl, ll in live_diffs:
        if n not in pinned:
            failures.append(f"new value divergence: {n}: gcc={gl!r} lccc={ll!r}")
        elif pinned[n] != (gl, ll):
            failures.append(
                f"changed value divergence: {n}: pinned={pinned[n]!r} live=({gl!r}, {ll!r})"
            )
    for n in pinned:
        if n not in live_map:
            failures.append(f"stale VALUE_DIFFS pin ({n} now agrees): update budgets")

    if failures:
        for line in failures:
            print(f"ERROR: {line}", file=sys.stderr)
        return 1
    print(
        f"macro parity: PASS (GCC {len(g)} / LCCC {len(l)} macros; "
        f"budgets {len(GCC_ONLY)} gcc-only, {len(LCCC_ONLY)} lccc-only, "
        f"{len(VALUE_DIFFS)} value diffs)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
