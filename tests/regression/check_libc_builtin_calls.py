#!/usr/bin/env python3
"""Check call-site arity against GCC 16.2, without linking malformed calls."""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


COPY = "void f(char *d, const char *s) { memcpy(d, s); }"
ARITY_ERROR = re.compile(
    r":(\d+):\d+: error: (too (?:few|many) arguments to function '\w+'; "
    r"expected (?:at least )?\d+, have \d+)"
)

# Source, flags, count of hard arity errors. Requiring positive and negative
# cases guards against rejecting whole classes of legacy C.
CASES = (
    ("first", COPY, (), 1),
    ("later", "void f(char *d, const char *s) {\n"
              "  memcpy(d, s, 1);\n  memcpy(d, s);\n  memcpy(d, s, 1, 2);\n}", (), 2),
    ("another-function", "void g(char *d, const char *s) { memcpy(d, s, 1); }\n" + COPY, (), 1),
    ("after-an-error", "void f(char *d, const char *s) {\n"
                       "  memcpy(d, s);\n  memcpy(d, s, 1, 2);\n}", (), 2),
    ("blanket", COPY, ("-fno-builtin",), 0),
    ("freestanding", COPY, ("-ffreestanding",), 0),
    ("specific", COPY, ("-fno-builtin-memcpy",), 0),
    ("other-specific", COPY, ("-fno-builtin-memset",), 1),
    ("last-restore", COPY, ("-fno-builtin", "-fbuiltin"), 1),
    ("sticky-specific", COPY, ("-fno-builtin-memcpy", "-fbuiltin"), 0),
    ("freestanding-restore", COPY, ("-ffreestanding", "-fbuiltin"), 1),
    ("old-style", "void f(void *p) { free(p); }\n"
                  "extern void free();\nvoid g(void *p) { free(p, p); }", (), 0),
    ("shadow", "void f(char *d, const char *s) {\n  memcpy(d, s, 1);\n"
               "  { int (*memcpy)(void *) = 0; (void)sizeof memcpy; }\n"
               "  memcpy(d, s);\n}", (), 1),
    ("function-pointer", "int f(int (*memcpy)(int, int)) { return memcpy(1, 2); }", (), 0),
    ("proper-declaration", "extern void *memcpy(void *, const void *, __SIZE_TYPE__);\n"
                           "void f(char *d, const char *s) { memcpy(d, s, 1); }", (), 0),
    ("variadic", 'void f(void) {\n  printf("%d", 42);\n  printf();\n}', (), 1),
    ("unchecked-bare", 'void f(void) { puts(); }', (), 0),
    ("explicit-puts", 'void f(void) { __builtin_puts(); }', (), 1),
    ("explicit-still-checked", 'void f(char *d, const char *s) { __builtin_memcpy(d, s); }',
     ("-ffreestanding", "-fno-builtin-memcpy"), 1),
    ("explicit-shadow", "int f(int (*__builtin_abs)(int, int)) { return __builtin_abs(1, 2); }", (), 0),
    ("reserved-bare", "void f(char *p) { __clear_cache(p); }", (), 1),
    ("reserved-withdrawn", "void f(char *p) { __clear_cache(p); }", ("-fno-builtin",), 0),
)


def run(compiler: str, source: Path, flags: tuple[str, ...], mode: str) -> tuple[int, str]:
    argv = [compiler, "-std=gnu90", mode, *flags, str(source)]
    if mode == "-S":
        argv += ["-o", os.devnull]
    result = subprocess.run(argv, capture_output=True, text=True, timeout=30,
                            env=dict(os.environ, LC_ALL="C", GCC_COLORS=""))
    return result.returncode, result.stderr


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", required=True)
    parser.add_argument("--gcc", default="gcc-16")
    args = parser.parse_args()
    version = subprocess.check_output([args.gcc, "-dumpfullversion"], text=True, timeout=30).strip()
    if version != "16.2.0":
        sys.exit(f"FAIL: expected GCC 16.2.0, found {version}")
    failures = []
    with tempfile.TemporaryDirectory(prefix="lccc-libc-calls.") as work:
        for case, text, flags, count in CASES:
            source = Path(work, f"{case}.c")
            source.write_text(text + "\n", encoding="utf-8")
            ref_rc, ref_diag = run(args.gcc, source, flags, "-fsyntax-only")
            expected = ARITY_ERROR.findall(ref_diag)
            if ref_rc != int(count > 0) or len(expected) != count:
                failures.append(f"{case}: unexpected oracle result ({ref_rc}):\n{ref_diag}")
                continue
            for mode in ("-fsyntax-only", "-S"):
                rc, diag = run(args.lccc, source, flags, mode)
                if rc != ref_rc or ARITY_ERROR.findall(diag) != expected:
                    failures.append(f"{case} {mode}: expected {ref_rc}, {expected}; got {rc}:\n{diag}")
    if failures:
        sys.exit("\n".join(failures))
    print(f"PASS: {len(CASES)} libc builtin cases against GCC {version}, in both driver paths")
    return 0


if __name__ == "__main__":
    sys.exit(main())
