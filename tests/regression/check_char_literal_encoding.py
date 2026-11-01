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
    # --- unknown escapes: accepted, but never silently ---------------------
    # GCC warns with no label, so only `-w` and `-Werror` act on it; the
    # character itself is kept, and `\%`, `\[` and `\{` are the measured
    # exceptions that stay silent.
    (r"'\q'", NARROW, ()),
    (r"'\8'", NARROW, ()),
    (r"'\9'", NARROW, ()),
    (r"'\-'", NARROW, ()),
    (r"'\%'", NARROW, ()),
    (r"'\['", NARROW, ()),
    (r"'\{'", NARROW, ()),
    (r"'\?'", NARROW, ()),
    (r"'\ '", NARROW, ()),
    (r"'\q\q'", NARROW, ()),
    (r"u'\q'", NARROW, ()),
    (r"U'\-'", NARROW, ()),
    (r"L'\q'", NARROW, ()),
    (r"u8'\q'", C23, ()),
    # --- a rejected escape contributes no character ------------------------
    # `'\xg'` is a ONE-character constant for GCC, so it carries no
    # multi-character diagnostic at all; the closing `}` of `'\x{zz}'` is an
    # ordinary character and is counted.
    (r"'\xg'", NARROW, ()),
    (r"'\xgabde'", NARROW, ()),
    (r"'\x\377'", NARROW, ()),
    (r"'\x{}'", NARROW, ()),
    (r"'\x{zz}'", NARROW, ()),
    (r"'\x{zzzzz}'", NARROW, ()),
    (r"'\o{}'", NARROW, ()),
    (r"'\o{8}'", NARROW, ()),
    (r"'\u{}'", NARROW, ()),
    (r"'\N{}'", NARROW, ()),
    (r"'\N{NO SUCH NAME}'", NARROW, ()),
    (r"'\N{NO SUCH NAME}zzzz'", NARROW, ()),
    (r"u'\N{NO SUCH NAME}'", NARROW, ()),
    (r"u8'\N{NO SUCH NAME}'", C23, ()),
    (r"'\N{zzzzzzzz'", NARROW, ()),
    (r"'\xe'", NARROW, ()),
    # --- `\N` without its braces is `\u` to GCC -------------------------
    (r"'\Na'", NARROW, ()),
    (r"'\Nabc'", NARROW, ()),
    (r"'\Nabcd'", NARROW, ()),
    (r"'\Nabcz'", NARROW, ()),
    (r"'\Nabczzzzzzz'", NARROW, ()),
    # --- a code point above U+10FFFF in `u''` -----------------------------
    (r"u'\U00110000a'", NARROW, ()),
    (r"u'a\U00110000'", NARROW, ()),
    (r"u'\U00110000\U00110000'", NARROW, ()),
    (r"u'\n\x1FF\U00110000Z'", NARROW, ()),
    (r"u'\U00200000\x41\x100#\u0\U00010000\400'", NARROW, ()),
    (r"U'\U00110000a'", NARROW, ()),
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
    (
        "char *s = u'\\U7FFFFFFF\\U0001F600';",
        (),
        [
            "warning: \\U7FFFFFFF is outside the UCS codespace",
            "error: converting UCN to execution character set: Invalid or "
            "incomplete multibyte or wide character",
            "warning: character not encodable in a single code unit",
        ],
        """GCC 16.2 reports the encoding prefix for this literal where LCCC
        reports the encodability of the astral character. The escape above
        U+10FFFF contributes no character to either compiler, so the literal
        holds one character; GCC nevertheless counts the SOURCE characters for
        its prefix rule when an unconvertible escape precedes an astral
        character, and no later diagnostic depends on the difference. Every
        other combination of the two is byte-identical, so this is pinned as a
        decision rather than left as a surprise (backlog GP-31).""",
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


# ---------------------------------------------------------------------------
# Generated sweep.
#
# The table above is curated: every row was a decision. A curated table can
# only pin what someone thought of, so `--sweep N` crosses the same atoms
# mechanically (`N` deterministic literals, seeded) and compares them the same
# way. It batches 25 literals per translation unit because LCCC renders at most
# 200 errors and 200 warnings per unit; each literal is on its own line, so the
# diagnostics map back by line number.
# ---------------------------------------------------------------------------

SWEEP_ATOMS = [
    "a", "Z", "0", " ", "#", "é", "ß", "€", "漢", "😀",
    r"\n", r"\t", r"\a", r"\v", r"\f", r"\r", r"\0", r"\?", r"\'", r"\\",
    r"\7", r"\77", r"\377", r"\400", r"\777", r"\3777", r"\8",
    r"\x41", r"\xff", r"\x100", r"\x1FF", r"\xg",
    r"\u0041", r"\u00e9", r"\u20ac", r"\uFFFF", r"\u0",
    r"\U0001F600", r"\U00010000", r"\U0010FFFF", r"\U00110000",
    r"\U7FFFFFFF", r"\U80000000", r"\U00200000",
    r"\N{SNOWMAN}", r"\N{DIGIT ONE}", r"\N{NO SUCH NAME}", r"\N",
    r"\x{41}", r"\x{1FF}", r"\o{777}", r"\u{1F600}", r"\q",
]
SWEEP_PREFIXES = ["", "u8", "u", "U", "L"]
SWEEP_DIALECTS = ("c23", "gnu23", "gnu17")


def sweep_literals(count: int, seed: int) -> list[tuple[str, str]]:
    """`count` deterministic literals, covering every atom and prefix once."""
    import random

    # `u8'x'` is a C23 literal: in C17 GCC reads `u8` as an ordinary
    # identifier, which is a lexical difference (documented, and pinned in the
    # curated table), not a character-constant one.
    entries = [
        (f"{prefix}'{atom}'", dialect)
        for prefix in SWEEP_PREFIXES
        for atom in SWEEP_ATOMS
        for dialect in SWEEP_DIALECTS
        if prefix != "u8" or dialect != "gnu17"
    ]
    rng = random.Random(seed)
    seen = set(entries)
    while len(entries) < count:
        prefix = rng.choice(SWEEP_PREFIXES)
        body = "".join(rng.choice(SWEEP_ATOMS) for _ in range(rng.randint(1, 7)))
        dialect = rng.choice(SWEEP_DIALECTS)
        if prefix == "u8" and dialect == "gnu17":
            continue
        entry = (f"{prefix}'{body}'", dialect)
        if entry in seen:
            continue
        seen.add(entry)
        entries.append(entry)
    return entries


# One class of literal is exempt from the sweep's strict comparison: a `u'…'`
# constant that mixes a code point above U+10FFFF (which no compiler can turn
# into a UTF-16 unit, so it contributes no character to either) with an astral
# character (two code units of one). Both compilers reject the literal; GCC
# reports the encoding prefix where LCCC reports the encodability of the astral
# character. The curated table pins one representative of the class (GP-31), and
# the sweep reports the class separately instead of counting it as a pass.
def is_pinned_u16_mix(literal: str) -> bool:
    if not literal.startswith("u'"):
        return False
    values = [int(match, 16) for match in re.findall(r"\\U([0-9A-Fa-f]{8})", literal)]
    values += [ord(ch) for ch in literal if ord(ch) > 0xFFFF]
    return any(value > 0x10FFFF for value in values) and any(
        0x1_0000 <= value <= 0x10FFFF for value in values
    )


def _diagnostics_by_line(stderr: str) -> dict[int, list[str]]:
    by_line: dict[int, list[str]] = {}
    for line in stderr.splitlines():
        match = re.match(r"^[^:]+:(\d+):(?:\d+:)? (.*)$", line)
        if match is None:
            continue
        text = match.group(2)
        if is_driver_summary(text):
            continue
        by_line.setdefault(int(match.group(1)), []).append(text)
    return by_line


def run_sweep(ccc: str, gcc: str, count: int, seed: int, verbose: bool) -> tuple[int, int]:
    """Compare `count` generated literals; returns (checked, failures)."""
    work = sweep_literals(count, seed)
    compilers = {"gcc": [gcc, "-fmax-errors=0"], "lccc": [ccc]}
    chunk = 25
    # One dialect per translation unit: the literals are lexed differently.
    batches = []
    for dialect in SWEEP_DIALECTS:
        same = [entry for entry in work if entry[1] == dialect]
        for base in range(0, len(same), chunk):
            batches.append(same[base:base + chunk])
    failures = 0
    checked = 0
    pinned = 0
    with tempfile.TemporaryDirectory() as tmp:
        for piece in batches:
            dialect = piece[0][1]
            body = ["int main(void) {"]
            for literal, _ in piece:
                body.append(f"    (void)({literal});")
            body += ["    return 0;", "}"]
            text = "\n".join(body) + "\n"
            diagnostic: dict[str, dict[int, list[str]]] = {}
            for label, command in compilers.items():
                source = os.path.join(tmp, f"diag-{label}.c")
                with open(source, "w", encoding="utf-8") as handle:
                    handle.write(text)
                proc = subprocess.run(
                    command + [f"-std={dialect}", "-fsyntax-only", source],
                    capture_output=True,
                    text=True,
                )
                diagnostic[label] = _diagnostics_by_line(proc.stderr)
            accepted = []
            for offset, (literal, _) in enumerate(piece):
                line = offset + 2  # line 1 is `int main(void) {`
                expected = diagnostic["gcc"].get(line, [])
                actual = diagnostic["lccc"].get(line, [])
                if is_pinned_u16_mix(literal):
                    pinned += 1
                    if verbose:
                        print(f"ok   (sweep, pinned deviation) {literal} [{dialect}]")
                    continue
                checked += 1
                if expected != actual:
                    failures += 1
                    print(f"FAIL (sweep) {literal} [{dialect}]")
                    print(f"      oracle: {expected}")
                    print(f"        lccc: {actual}")
                elif all("error:" not in message for message in expected):
                    accepted.append((offset, literal))
            if not accepted:
                continue
            body = ["#include <stdio.h>", "int main(void) {"]
            for offset, literal in accepted:
                body.append(
                    f'    printf("{offset} %llX %zu\\n", '
                    f"(unsigned long long)(long long)({literal}), sizeof({literal}));"
                )
            body += ["    return 0;", "}"]
            text = "\n".join(body) + "\n"
            measured: dict[str, dict[int, str]] = {}
            for label, command in compilers.items():
                source = os.path.join(tmp, f"value-{label}.c")
                binary = os.path.join(tmp, f"value-{label}")
                with open(source, "w", encoding="utf-8") as handle:
                    handle.write(text)
                built = subprocess.run(
                    command + [f"-std={dialect}", source, "-o", binary],
                    capture_output=True,
                    text=True,
                )
                if built.returncode != 0:
                    measured[label] = {}
                    continue
                ran = subprocess.run([binary], capture_output=True, text=True)
                measured[label] = {
                    int(parts[0]): " ".join(parts[1:])
                    for parts in (line.split() for line in ran.stdout.splitlines())
                    if parts
                }
            for offset, literal in accepted:
                if measured["gcc"].get(offset) != measured["lccc"].get(offset):
                    failures += 1
                    print(f"FAIL (sweep value) {literal} [{dialect}]")
                    print(f"      oracle: {measured['gcc'].get(offset)!r}")
                    print(f"        lccc: {measured['lccc'].get(offset)!r}")
                elif verbose:
                    print(f"ok   (sweep) {literal} [{dialect}]")
    if pinned:
        print(f"sweep: {pinned} literal(s) in the pinned u''/astral class (GP-31)")
    return checked, failures


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
    parser.add_argument(
        "--sweep",
        type=int,
        default=0,
        metavar="N",
        help="additionally compare N generated literals (deterministic seed), "
        "batched one translation unit per compiler chunk",
    )
    parser.add_argument("--sweep-seed", type=int, default=7)
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

    if args.sweep:
        swept, sweep_failures = run_sweep(
            args.ccc, args.gcc, args.sweep, args.sweep_seed, args.verbose
        )
        checks += swept
        failures += sweep_failures

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
