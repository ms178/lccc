#!/usr/bin/env python3
"""Differential C-feature probe: lccc versus GCC 16.2 and Clang 23.1.4.

Every case is one self-contained C program with a defined exit status. The
probe compiles it with each compiler, runs the binary, and reports the matrix
cell:

    Rnn   compiled, linked, ran, exited with status nn
    C     compile or link failed
    T     compile or run timed out
    X     the binary could not be executed at all

Why a probe rather than a feature list: a language feature is "supported"
when the emitted program behaves like the reference compiler's, and the only
honest way to know that is to build and run the same source three ways. This
is the replay corpus for the GCC 16.2 parity campaign
(docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md): each divergence found here is
either fixed with a regression fixture under tests/regression/ or recorded in
the campaign register with a reason.

A case is a GAP when lccc rejects source GCC accepts, or when both build but
the exit statuses differ. lccc *accepting* what GCC rejects is a permissive
extension, reported as PERMISSIVE and not counted as a gap.

Usage::

    python3 scripts/gcc16_feature_probe.py
    python3 scripts/gcc16_feature_probe.py --filter ucn --filter f128 -v
    python3 scripts/gcc16_feature_probe.py --gcc gcc-16 --clang clang-23
    python3 scripts/gcc16_feature_probe.py --json /tmp/probe.json

The compilers are taken from ``--lccc/--gcc/--clang``, then ``$LCCC``,
``$PROBE_GCC``, ``$PROBE_CLANG``, then the agent toolchain cache, then PATH.
No third-party packages. Exit status is 0 unless ``--fail-on-gap`` is given
and a gap was found.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
TOOLCHAIN_CACHE = Path.home() / ".cache" / "lccc" / "agent-toolchains" / "bin"

# (name, source). Keep each snippet a complete program returning 0 on success:
# the exit status is the verdict, so a case must fail loudly when the feature
# is miscompiled rather than merely absent.
CASES = [
    # ---- C23 core language ----
    ("bitint-17", "int main(void){ _BitInt(17) x = 100000; return (int)(x % 7); }"),
    ("bitint-unsigned-128",
     "int main(void){ unsigned _BitInt(128) x = 1; x <<= 100; return (int)(x >> 100); }"),
    ("bitint-add", "int main(void){ _BitInt(7) a = 100, b = 27; _BitInt(7) c = a + b; return (int)c != (int)((a + b) & 0x7f); }"),
    ("typeof-unqual", "int main(void){ const int a = 5; typeof_unqual(a) b = 6; b++; return b - 7; }"),
    ("constexpr-var", "int main(void){ constexpr int a = 3; static_assert(a == 3); return a - 3; }"),
    ("auto-infer", "int main(void){ auto x = 41; return x + 1 - 42; }"),
    ("nullptr", "#include <stddef.h>\nint main(void){ nullptr_t p = nullptr; return p == nullptr ? 0 : 1; }"),
    ("bool-kw", "int main(void){ bool b = true; return b ? 0 : 1; }"),
    ("enum-fixed", "enum E : unsigned char { A = 200 }; int main(void){ enum E e = A; return (int)e - 200; }"),
    ("attr-c23", "int main(void){ [[maybe_unused]] int x = 1; [[nodiscard]] int f(void); return 0; }"),
    ("attr-fallthrough", "int main(void){ int x=2; switch(x){ case 1: x=3; [[fallthrough]]; case 2: x=4; } return x-4; }"),
    ("elifdef", "#define A\nint main(void){\n#ifdef A\n#elifdef B\n return 1;\n#else\n return 0;\n#endif\n}"),
    ("hash-warning", "#warning hello\nint main(void){return 0;}"),
    ("va-opt", "int f(int a, ...){return a;}\n#define G(a, ...) f(a __VA_OPT__(,) __VA_ARGS__)\nint main(void){ return G(0) + G(0, 1) - 0; }"),
    ("u8-char", "int main(void){ return (int)u8'a' - 97; }"),
    ("digit-sep", "int main(void){ return (int)(1'000'000 % 7); }"),
    ("binary-lit", "int main(void){ return (int)(0b1010 - 10); }"),
    ("char8-str", "int main(void){ unsigned char const *s = u8\"ab\"; return s[0]=='a'?0:1; }"),
    ("char8-type", "#include <uchar.h>\nint main(void){ char8_t c = u8'a'; return (int)c - 97; }"),
    # ---- GNU extensions ----
    ("stmt-expr", "int main(void){ int x = ({ int y = 40; y + 2; }); return x - 42; }"),
    ("nested-fn", "int main(void){ int f(int n){ return n+1; } return f(41) - 42; }"),
    ("label-values", "int main(void){ void *p = &&L; goto *p; L: return 0; }"),
    ("cleanup",
     "static void d(int *p){ *p = 7; } int main(void){ int v = 0; { __attribute__((cleanup(d))) int *q = &v; (void)q; } return v - 7; }"),
    ("ctor-var-ignored",
     "static int x; static void c(void){ x = 5; } void (*fp)(void) __attribute__((constructor)) = c; int main(void){ return x - 5; }"),
    ("ctor-fn",
     "static int x; static void c(void) __attribute__((constructor)); static void c(void){ x = 5; } int main(void){ return x - 5; }"),
    ("dtor-fn",
     "static int x; static void c(void) __attribute__((destructor)); static void c(void){ x = 5; } int main(void){ return x - 5; }"),
    ("vector-size", "int main(void){ typedef int v4 __attribute__((vector_size(16))); v4 a = {1,2,3,4}; return a[3] - 4; }"),
    ("mode-attr", "int main(void){ typedef int __attribute__((mode(QI))) qi; qi x = 300; return (int)x - 44; }"),
    ("transparent-union",
     "typedef union __attribute__((transparent_union)) { int *i; long *l; } U; int f(U u){return 0;} int main(void){ int v=0; return f(&v); }"),
    ("case-range", "int main(void){ int x=5; switch(x){ case 1 ... 9: return 0; default: return 1; } }"),
    ("elvis", "int main(void){ int a = 0, b = 42; return (a ?: b) - 42; }"),
    ("designated-range", "int main(void){ int a[10] = { [0 ... 9] = 3 }; return a[9] - 3; }"),
    ("auto-type", "int main(void){ __auto_type x = 42; return x - 42; }"),
    ("extension-kw", "int main(void){ return __extension__ 0; }"),
    ("asm-goto", "int main(void){ int r = 0; asm goto(\"\" :::: l); return 1; l: return r; }"),
    ("label-decl", "int main(void){ __label__ l; l: ; return 0; }"),
    ("zero-len-array",
     "struct S { int n; int a[0]; }; int main(void){ struct S s; s.n = 0; return (int)__builtin_offsetof(struct S, a) - 4; }"),
    # ---- types ----
    ("int128", "int main(void){ __int128 x = 1; x <<= 70; return (int)(x >> 70); }"),
    ("float16", "int main(void){ _Float16 h = 1.5; return (int)(h * 2) - 3; }"),
    ("float128", "int main(void){ _Float128 f = 1.5Q; return (int)(f * 2) - 3; }"),
    ("float128-precision",
     "int main(void){ volatile _Float128 a = 1.0Q, b = 1e-20Q; return (a + b) == a ? 1 : 0; }"),
    ("float32", "int main(void){ _Float32 f = 1.5F32; return (int)(f*2) - 3; }"),
    ("decimal32", "int main(void){ _Decimal32 d = 1.5df; return (int)(d*2) - 3; }"),
    ("complex", "int main(void){ _Complex double c = 1.0 + 2.0i; return (int)(__real__ c) - 1; }"),
    ("fp16", "int main(void){ __fp16 h = 1.5; return (int)h - 1; }"),
    # ---- builtins ----
    ("builtin-choose", "int main(void){ return __builtin_choose_expr(1, 0, 1); }"),
    ("builtin-types", "int main(void){ return __builtin_types_compatible_p(int, int) ? 0 : 1; }"),
    ("builtin-convertvector",
     "typedef int v4 __attribute__((vector_size(16))); typedef float f4 __attribute__((vector_size(16))); int main(void){ v4 a={1,2,3,4}; f4 b=__builtin_convertvector(a,f4); return (int)b[1]-2; }"),
    ("builtin-overflow", "int main(void){ int r; if(!__builtin_add_overflow(1,2,&r)) return r-3; return 1; }"),
    ("builtin-bswap128", "int main(void){ unsigned __int128 x = 1; x = __builtin_bswap128(x); return (int)(x >> 127); }"),
    ("builtin-countl-ll", "int main(void){ return (int)__builtin_clzll(1ULL) - 63; }"),
    ("builtin-isnan-f128", "int main(void){ _Float128 f = 0.0Q/0.0Q; return __builtin_isnan(f) ? 0 : 1; }"),
    ("builtin-isinf-f128", "int main(void){ _Float128 f = 1.0Q/0.0Q; return __builtin_isinf(f) ? 0 : 1; }"),
    ("builtin-isfinite-f128", "int main(void){ _Float128 f = 1.0Q/0.0Q; return __builtin_isfinite(f) ? 1 : 0; }"),
    ("builtin-fpclassify-f128",
     "int main(void){ _Float128 z = 0.0Q, n = z/z; return __builtin_fpclassify(0,1,2,3,4,n); }"),
    ("builtin-isnan-longdouble", "int main(void){ long double z = 0.0L, n = z/z; return __builtin_isnan(n) ? 0 : 1; }"),
    # ---- attributes registry ----
    ("attr-retain", "static int x __attribute__((retain)) = 1; int main(void){ return x-1; }"),
    ("attr-noinit", "int x __attribute__((noinit)); int main(void){ return x - x; }"),
    ("attr-access",
     "int f(int *p, int n) __attribute__((access(read_only,1,n))); int f(int *p,int n){return p?n:0;} int main(void){ int v=1; return f(&v,0); }"),
    ("attr-counted-by", "struct S { int n; int a[] __attribute__((counted_by(n))); }; int main(void){ return 0; }"),
    ("attr-nonnull", "int g(int *p) __attribute__((nonnull)); int g(int *p){ return p?0:1; } int main(void){ int v=0; return g(&v); }"),
    ("attr-unavailable", "int f(void) __attribute__((unavailable)); int main(void){ return 0; }"),
    ("attr-warn-unused", "int f(void) __attribute__((warn_unused_result)); int f(void){return 0;} int main(void){ return f(); }"),
    ("attr-alias-fn", "static int impl(void){return 0;} int alias(void) __attribute__((alias(\"impl\"))); int main(void){ return alias(); }"),
    ("attr-section", "static int x __attribute__((section(\".mydata\"))) = 1; int main(void){ return x-1; }"),
    ("attr-target", "int main(void){ return 0; } __attribute__((target(\"avx2\"))) int t(void){ return 0; }"),
    # ---- macros / preprocessor ----
    ("has-builtin", "int main(void){ return __has_builtin(__builtin_add_overflow) ? 0 : 1; }"),
    ("has-c-attribute", "int main(void){ return __has_c_attribute(nodiscard) ? 0 : 1; }"),
    ("has-include",
     "#if __has_include(<stdio.h>)\nint main(void){ return 0; }\n#else\n#error stdio.h not found\n#endif"),
    ("ucn-ident", "int \\u00e9 = 1; int main(void){ return \\u00e9 - 1; }"),
    ("ucn-string-utf8", "int main(void){ char *s = \"\\u00e9\"; return (unsigned char)s[0] == 0xC3 && (unsigned char)s[1] == 0xA9 ? 0 : 1; }"),
    ("ucn-vs-hex-bytes",
     "int main(void){ return sizeof(\"\\u00e9\") == 3 && sizeof(\"\\xe9\") == 2 ? 0 : 1; }"),
    # C23 named universal character escapes (a C++23 feature GCC and Clang also
    # accept in C). The five cases pin the three name sources: the front-coded
    # explicit table, a hex-suffix family, and a composed Hangul syllable.
    ("ucn-named-explicit",
     "int main(void){ char *s = \"\\N{LATIN SMALL LETTER E WITH ACUTE}\"; return (unsigned char)s[0] == 0xC3 && (unsigned char)s[1] == 0xA9 ? 0 : 1; }"),
    ("ucn-named-family",
     "int main(void){ char *s = \"\\N{CJK UNIFIED IDEOGRAPH-4E00}\"; return sizeof(\"\\N{CJK UNIFIED IDEOGRAPH-4E00}\") == 4 && (unsigned char)s[0] == 0xE4 ? 0 : 1; }"),
    ("ucn-named-hangul",
     "int main(void){ char *s = \"\\N{HANGUL SYLLABLE GA}\"; return sizeof(\"\\N{HANGUL SYLLABLE GA}\") == 4 && (unsigned char)s[0] == 0xEA ? 0 : 1; }"),
    ("ucn-named-wide",
     "int main(void){ return U\"\\N{GRINNING FACE}\"[0] == 0x1F600 && L\"\\N{SNOWMAN}\"[0] == 0x2603 ? 0 : 1; }"),
    ("ucn-named-charliteral",
     "int main(void){ return (int)'\\N{SNOWMAN}' == 0xE29883 ? 0 : 1; }"),
    # ---- diagnostics-driven acceptance ----
    ("kr-def", "int f(a) int a; { return a; } int main(void){ return f(0); }"),
    ("implicit-int", "f(void){ return 0; } int main(void){ return 0; }"),
]


def first_existing(*candidates: str | None) -> str | None:
    for candidate in candidates:
        if not candidate:
            continue
        path = Path(candidate)
        if path.is_absolute() and path.exists():
            return str(path)
        import shutil

        found = shutil.which(candidate)
        if found:
            return found
    return None


def resolve_tools(args) -> dict[str, list[str] | None]:
    lccc = first_existing(
        args.lccc or os.environ.get("LCCC"), str(REPO_ROOT / "target/fastbuild/lccc"), "lccc"
    )
    gcc = first_existing(
        args.gcc or os.environ.get("PROBE_GCC"), str(TOOLCHAIN_CACHE / "gcc-16"), "gcc-16"
    )
    clang = first_existing(
        args.clang or os.environ.get("PROBE_CLANG"), str(TOOLCHAIN_CACHE / "clang-23"), "clang-23"
    )
    return {"lccc": [lccc] if lccc else None, "gcc": [gcc] if gcc else None,
            "clang": [clang] if clang else None}


def run_case(tool_cmd: list[str], source: str, workdir: Path, extra: list[str]) -> tuple[str, str]:
    """Compile, link and run one case; return (cell, diagnostic text)."""
    case_c = workdir / "case.c"
    case_c.write_text(source)
    exe = workdir / "case.bin"
    cmd = tool_cmd + [str(case_c), "-o", str(exe), "-w", *extra]
    try:
        compiled = subprocess.run(cmd, capture_output=True, text=True, timeout=90)
    except subprocess.TimeoutExpired:
        return "T", "compile timed out"
    if compiled.returncode != 0:
        detail = (compiled.stderr or compiled.stdout).strip().splitlines()[:3]
        return "C", " | ".join(detail)
    try:
        ran = subprocess.run([str(exe)], capture_output=True, timeout=60)
    except subprocess.TimeoutExpired:
        return "T", "run timed out"
    except OSError as exc:
        return "X", str(exc)
    if ran.returncode < 0:
        return f"R{ran.returncode}", f"killed by signal {-ran.returncode}"
    return f"R{ran.returncode}", ""


def classify(cells: dict[str, str]) -> tuple[str, bool]:
    """Return (verdict, is_gap) for one case."""
    lccc, gcc = cells["lccc"], cells["gcc"]
    lccc_broken = lccc.startswith(("C", "T", "X"))
    gcc_broken = gcc is None or gcc.startswith(("C", "T", "X"))
    if lccc_broken and not gcc_broken:
        return "LCCC-REJECTS", True
    if not lccc_broken and gcc_broken:
        return "PERMISSIVE", False
    if lccc == gcc:
        return "=", False
    return "DIFF", True


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", help="lccc binary (default: $LCCC, then target/fastbuild/lccc)")
    parser.add_argument("--gcc", help="GCC oracle (default: $PROBE_GCC, then gcc-16)")
    parser.add_argument("--clang", help="Clang oracle (default: $PROBE_CLANG, then clang-23)")
    parser.add_argument("--filter", action="append", default=[],
                        help="run only cases whose name contains this substring (repeatable)")
    parser.add_argument("--extra-flag", action="append", default=[],
                        help="extra compiler flag for every tool (repeatable)")
    parser.add_argument("--jobs", type=int, default=8, help="parallel cases (default 8)")
    parser.add_argument("--json", help="write the full matrix as JSON here")
    parser.add_argument("--fail-on-gap", action="store_true",
                        help="exit non-zero when at least one gap was found")
    parser.add_argument("-v", "--verbose", action="store_true", help="print every diagnostic")
    args = parser.parse_args()

    tools = resolve_tools(args)
    if tools["lccc"] is None:
        print("error: no lccc binary found (build it or pass --lccc)", file=sys.stderr)
        return 2
    missing = [name for name, cmd in tools.items() if cmd is None and name != "clang"]
    if "gcc" in missing:
        print("error: no GCC oracle found (pass --gcc)", file=sys.stderr)
        return 2

    cases = [(name, src) for name, src in CASES
             if not args.filter or any(f in name for f in args.filter)]
    if not cases:
        print("error: no case matched the filter", file=sys.stderr)
        return 2

    def probe(item):
        name, src = item
        cells, details = {}, {}
        for tool in ("lccc", "gcc", "clang"):
            cmd = tools[tool]
            if cmd is None:
                cells[tool] = None
                continue
            with tempfile.TemporaryDirectory() as dirname:
                cells[tool], detail = run_case(cmd, src, Path(dirname), args.extra_flag)
            if detail:
                details[tool] = detail
        return name, cells, details

    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        rows = list(pool.map(probe, cases))

    name_width = max(len(name) for name, _ in cases) + 2
    header = f"{'case'.ljust(name_width)} {'lccc':<7} {'gcc':<7} {'clang':<7} verdict"
    print(header)
    print("-" * len(header))
    gaps, permissive = [], []
    for name, cells, details in rows:
        verdict, is_gap = classify(cells)
        if is_gap:
            gaps.append(name)
        elif verdict == "PERMISSIVE":
            permissive.append(name)
        print(f"{name.ljust(name_width)} {str(cells['lccc'] or '-'):<7} "
              f"{str(cells['gcc'] or '-'):<7} {str(cells['clang'] or '-'):<7} {verdict}")
        if args.verbose:
            for tool, detail in details.items():
                print(f"{'':<{name_width}} {tool}: {detail}")

    print()
    print(f"{len(cases)} cases, {len(gaps)} gap(s), {len(permissive)} permissive divergence(s)")
    if gaps:
        print("gaps: " + ", ".join(gaps))

    if args.json:
        payload = {"cases": [
            {"name": name, "cells": cells, "details": details,
             "verdict": classify(cells)[0]}
            for name, cells, details in rows
        ], "gaps": gaps, "permissive": permissive}
        Path(args.json).write_text(json.dumps(payload, indent=2) + "\n")

    return 1 if (gaps and args.fail_on_gap) else 0


if __name__ == "__main__":
    raise SystemExit(main())
