#!/usr/bin/env python3
"""Differential check of character-constant encoding and diagnostics.

WHY: A character constant is the one place in C where the *type* of the literal
decides what its text means, and every branch of that decision is silent when it
is wrong:

  * `'\\u00e9'` is a two-character constant (the UTF-8 bytes C3 A9) while
    `'\\xe9'` is the single byte E9 — the distinction is the escape's *form*,
    not the value's magnitude;
  * `u'\\U0001F600'` is 0xDE00, the LOW SURROGATE of the pair, not the low 16
    bits of the code point (0xF600) and not the high surrogate;
  * `u8'\\u00e9'` must be rejected outright in C23 (`character not encodable in
    a single code unit`) while `u8'\\xe9'` is fine;
  * `u'ab'` keeps its LAST character (0x62) while the narrow `'ab'` accumulates
    its bytes (0x6162);
  * the width of an element decides whether `\\x100` is out of range, so the
    same escape warns in `'…'` and `u8'…'` but is clean in `u'…'` and `L'…'`;
  * `''` is an error, and five-character constants warn unconditionally while
    two- to four-character ones are `-Wmultichar` (which `-Wno-multichar`
    silences and `-Werror=multichar` promotes).

Pinning those numbers by hand would only ever pin what LCCC already believes, so
this check compiles every case with both compilers and compares the observable
result: the sequence of `warning:`/`error:` messages, the value of the constant
and its `sizeof`, measured by running the program the compiler produces. Every
expectation in the table is therefore GCC 16.2's behaviour, read off the oracle
rather than written down here.

The `DEVIATIONS` table at the end is the one place where the two compilers are
allowed to disagree, and it pins LCCC's side explicitly so a change is noticed.

Usage:
    CCC=target/fastbuild/lccc GCC=gcc-16 \
        python3 tests/regression/check_char_literal_encoding.py
"""

from __future__ import annotations

import argparse
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile

# ---------------------------------------------------------------------------
# The matrix.
#
# Each entry is (literal, dialects, extra flags). `dialects` is the set of
# `-std=` values in which the case is spelled the same way in both compilers:
# `u8'a'` is a C23 character literal, and in C17 GCC lexes `u8` as an ordinary
# identifier, which is a different (and documented) divergence.
# ---------------------------------------------------------------------------
ALL = ("c23", "gnu23", "gnu17")
NARROW = ("c23", "gnu23", "gnu17")
C23 = ("c23", "gnu23")

CASES = [
    # --- narrow constants: bytes, high byte first -------------------------
    ("'a'", NARROW, ()),
    ("'0'", NARROW, ()),
    ("' '", NARROW, ()),
    ("'ab'", NARROW, ()),
    ("'abc'", NARROW, ()),
    ("'abcd'", NARROW, ()),
    ("'abcde'", NARROW, ()),
    ("'abcdefgh'", NARROW, ()),
    ("'é'", NARROW, ()),
    ("'€'", NARROW, ()),
    ("''", NARROW, ()),
    # --- narrow constants: escapes are bytes ------------------------------
    (r"'\n'", NARROW, ()),
    (r"'\t'", NARROW, ()),
    (r"'\0'", NARROW, ()),
    (r"'\7'", NARROW, ()),
    (r"'\377'", NARROW, ()),
    (r"'\3777'", NARROW, ()),
    (r"'\777'", NARROW, ()),
    (r"'\400'", NARROW, ()),
    (r"'\x41'", NARROW, ()),
    (r"'\xff'", NARROW, ()),
    (r"'\x100'", NARROW, ()),
    (r"'\x1FF'", NARROW, ()),
    (r"'a\x100'", NARROW, ()),
    (r"'\400\uD800'", NARROW, ()),
    # --- narrow constants: universal character names are characters -------
    (r"'\u0041'", NARROW, ()),
    (r"'\u00e9'", NARROW, ()),
    (r"'\u20ac'", NARROW, ()),
    (r"'\U0001F600'", NARROW, ()),
    (r"'\N{SNOWMAN}'", NARROW, ()),
    (r"'\N{DIGIT ONE}'", NARROW, ()),
    # --- narrow constants: delimited (C2y) escapes ------------------------
    (r"'\x{41}'", NARROW, ()),
    (r"'\x{1FF}'", NARROW, ()),
    (r"'\o{7}'", NARROW, ()),
    (r"'\o{777}'", NARROW, ()),
    (r"'\x{}'", NARROW, ()),
    (r"'\o{}'", NARROW, ()),
    (r"'\u{}'", NARROW, ()),
    (r"'\x{z}'", NARROW, ()),
    (r"'\x{4z}'", NARROW, ()),
    (r"'\o{8}'", NARROW, ()),
    (r"'\o{7x}'", NARROW, ()),
    (r"'\u{z}'", NARROW, ()),
    (r"'\u{12z}'", NARROW, ()),
    (r"'\x'", NARROW, ()),
    (r"'\o'", NARROW, ()),
    # --- narrow constants: UCN validity ----------------------------------
    (r"'\u12'", NARROW, ()),
    (r"'\U1234'", NARROW, ()),
    (r"'\uD800'", NARROW, ()),
    (r"'\uD800a'", NARROW, ()),
    (r"'\UFFFFFFFF'", NARROW, ()),
    (r"'\U80000000a'", NARROW, ()),
    (r"'\U00110000'", NARROW, ()),
    (r"'\U001FFFFF'", NARROW, ()),
    (r"'\U00200000'", NARROW, ()),
    (r"'\U7FFFFFFF'", NARROW, ()),
    (r"'\U{110000}'", NARROW, ()),
    # --- u8: one UTF-8 code unit ------------------------------------------
    ("u8'a'", C23, ()),
    (r"u8'\xff'", C23, ()),
    (r"u8'\xe9'", C23, ()),
    (r"u8'\u0041'", C23, ()),
    (r"u8'\u00e9'", C23, ()),
    (r"u8'\u00ff'", C23, ()),
    (r"u8'\U0001F600'", C23, ()),
    (r"u8'\U0001F600a'", C23, ()),
    ("u8'ab'", C23, ()),
    ("u8''", C23, ()),
    (r"u8'\x100'", C23, ()),
    (r"u8'\x{100}'", C23, ()),
    (r"u8'\xc3\xa9a'", C23, ()),
    (r"u8'\uD800a'", C23, ()),
    (r"u8'\U80000000'", C23, ()),
    (r"u8'\U00110000'", C23, ()),
    # --- u: one UTF-16 code unit ------------------------------------------
    ("u'a'", NARROW, ()),
    (r"u'\u00e9'", NARROW, ()),
    (r"u'\uFFFF'", NARROW, ()),
    (r"u'\U00010000'", NARROW, ()),
    (r"u'\U0001F600'", NARROW, ()),
    (r"u'\U0010FFFF'", NARROW, ()),
    (r"u'\x100'", NARROW, ()),
    (r"u'\xFFFF'", NARROW, ()),
    (r"u'\x10000'", NARROW, ()),
    (r"u'\x{10000}'", NARROW, ()),
    ("u'ab'", NARROW, ()),
    ("u'abc'", NARROW, ()),
    (r"u'a\U0001F600'", NARROW, ()),
    (r"u'\U0001F600a'", NARROW, ()),
    (r"u'\U00110000'", NARROW, ()),
    (r"u'\U00110000a'", NARROW, ()),
    (r"u'\uD800'", NARROW, ()),
    (r"u'\U80000000'", NARROW, ()),
    ("u''", NARROW, ()),
    # --- U: one UTF-32 code unit ------------------------------------------
    ("U'a'", NARROW, ()),
    (r"U'\u00e9'", NARROW, ()),
    (r"U'\U0001F600'", NARROW, ()),
    (r"U'\U00110000'", NARROW, ()),
    (r"U'\U001FFFFF'", NARROW, ()),
    (r"U'\x100'", NARROW, ()),
    (r"U'\x100000000'", NARROW, ()),
    ("U'ab'", NARROW, ()),
    ("U''", NARROW, ()),
    (r"U'\U00110000a'", NARROW, ()),
    (r"U'\uD800\777'", NARROW, ()),
    # --- L: wchar_t, which is int on this target ---------------------------
    ("L'a'", NARROW, ()),
    (r"L'\u00e9'", NARROW, ()),
    (r"L'\U0001F600'", NARROW, ()),
    (r"L'\x100'", NARROW, ()),
    (r"L'\x100000000'", NARROW, ()),
    ("L'ab'", NARROW, ()),
    ("L''", NARROW, ()),
    # --- `-Wmultichar` is a real switch ------------------------------------
    ("'ab'", NARROW, ("-Wno-multichar",)),
    ("'ab'", NARROW, ("-Werror=multichar",)),
    ("'abcd'", NARROW, ("-Werror=multichar",)),
    ("'abcde'", NARROW, ("-Wno-multichar",)),
    ("'ab'", NARROW, ("-w",)),
    (r"'\u00e9'", NARROW, ("-Wno-multichar",)),
    ("u'ab'", NARROW, ("-Wno-multichar",)),
    ("u'\U0001F600'", NARROW, ("-Wno-multichar",)),
    (r"u'\U0001F600'", NARROW, ("-Werror",)),
    (r"u8'\u00e9'", C23, ("-w",)),
]

# ---------------------------------------------------------------------------
# Documented divergences.
#
# Every entry pins LCCC's own behaviour; the note records what GCC 16.2 does,
# measured, so the reason is not lost.
# ---------------------------------------------------------------------------
DEVIATIONS = [
    (
        'char *s = "\\U00110000";',
        (),
        ["error: \\U00110000 is outside the UCS codespace"],
        """GCC 16.2 accepts the string, keeping the generalized-UTF-8 bytes
        F4 90 80 80; LCCC warns with GCC's sentence and then rejects the
        literal, because a string literal's payload is a Rust `String` and
        cannot hold a code point above U+10FFFF (backlog GP-29). Character
        constants above U+10FFFF are fully supported.""",
    ),
]


def is_driver_summary(message: str) -> bool:
    """The driver's trailing count line (`…: N error(s)`) is not a diagnostic."""
    return (
        re.fullmatch(
            r"(?:[^:]*: )?\d+ (?:frontend )?error\(s\)(?: during semantic analysis)?",
            message,
        )
        is not None
    )


def diagnostics_of(stderr: str, source: str) -> list[str]:
    """The diagnostics in `stderr`, as `severity: message`, in order."""
    out = []
    for line in stderr.splitlines():
        match = re.search(r"(warning|error|note): (.*)", line)
        if match is None:
            continue
        if is_driver_summary(match.group(2)):
            continue
        out.append(f"{match.group(1)}: {match.group(2)}")
    return out


PROGRAM = """#include <stdio.h>
int main(void) {{
    printf("%llX %zu\\n", (unsigned long long)(long long)({literal}),
           sizeof({literal}));
    return 0;
}}
"""


def run_case(compiler: str, language: str, literal: str, flags) -> tuple[list[str], str]:
    """Compile `literal` with `compiler`, returning (diagnostics, value).

    One compile per side: the build's stderr carries the diagnostics the
    syntax-only run would print, and a build that fails is a rejection on both
    sides for every case in the matrix (none of them is a link error).
    """
    with tempfile.TemporaryDirectory() as work:
        source = os.path.join(work, "case.c")
        binary = os.path.join(work, "case")
        with open(source, "w", encoding="utf-8") as handle:
            handle.write(PROGRAM.format(literal=literal))
        built = subprocess.run(
            [compiler, f"-std={language}", *flags, source, "-o", binary],
            capture_output=True,
            text=True,
        )
        if built.returncode != 0:
            return diagnostics_of(built.stderr, source), "<rejected>"
        ran = subprocess.run([binary], capture_output=True, text=True)
        return diagnostics_of(built.stderr, source), ran.stdout.strip()


def main() -> int:
    repository = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--ccc", default=os.environ.get("CCC", os.path.join(repository, "target", "fastbuild", "lccc"))
    )
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument(
        "--jobs",
        type=int,
        default=min(8, os.cpu_count() or 1),
        help="compilers to run at once (the check is subprocess-bound)",
    )
    args = parser.parse_args()

    def check(entry) -> tuple[str, str | None]:
        literal, language, flags = entry
        label = f"{literal} [-std={language} {' '.join(flags)}]".replace(" ]", "]")
        expected = run_case(args.gcc, language, literal, flags)
        actual = run_case(args.ccc, language, literal, flags)
        if expected == actual:
            return label, None
        report = [f"FAIL {label}"]
        if expected[1] != actual[1]:
            report.append(f"       value: gcc {expected[1]}  lccc {actual[1]}")
        if expected[0] != actual[0]:
            report.append(f"      oracle: {expected[0]}")
            report.append(f"        lccc: {actual[0]}")
        return label, "\n".join(report)

    work = [
        (literal, language, flags)
        for literal, dialects, flags in CASES
        for language in dialects
    ]
    checks = len(work)
    failures = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        for label, report in pool.map(check, work):
            if report is None:
                if args.verbose:
                    print(f"ok   {label}")
                continue
            failures += 1
            print(report)

    for source, flags, expected, note in DEVIATIONS:
        checks += 1
        with tempfile.TemporaryDirectory() as work:
            path = os.path.join(work, "dev.c")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(source + "\n")
            result = subprocess.run(
                [args.ccc, "-std=gnu23", *flags, "-fsyntax-only", path],
                capture_output=True,
                text=True,
            )
            actual = diagnostics_of(result.stderr, path)
        if actual != expected:
            failures += 1
            print(f"FAIL (documented deviation) {source}")
            print(f"      expected: {expected}")
            print(f"          lccc: {actual}")
        elif args.verbose:
            print(f"ok   (documented deviation) {source}")

    print(f"checked {checks} case(s): {checks - failures} match, {failures} differ")
    if failures:
        print(
            "\nEvery failure is a disagreement with the oracle in the value of a\n"
            "character constant, its type, or the diagnostics GCC prints for it.\n"
            "See src/frontend/lexer/scan.rs (`lex_char`, `lex_wide_char`).",
            file=sys.stderr,
        )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
