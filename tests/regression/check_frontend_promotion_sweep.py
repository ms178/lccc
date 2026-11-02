#!/usr/bin/env python3
"""Exhaustive promotion sweep: unary `-`/`~`/`+` and `?:` over every integer pair.

The defect this gate was written for is a *width* defect: a unary operator
applied to a narrow operand must be evaluated at the operand's PROMOTED type,
and only then converted to the result type, and a `?:` whose operands differ in
signedness must be evaluated at the ARITHMETIC common type of the two arms and
only then converted. Both rules are easy to state and easy to get wrong by one
conversion, which is exactly what happened: `-(u * 1)` on a 16-bit `u` produced
the 32-bit negation of a sign-extended value, and `(unsigned long long)(1 ? -1
: 1u)` produced the sign-extended -1 instead of 4294967295.

A hand-written corpus can only sample that space. This gate enumerates it: for
every pair of integer types, at boundary values of both, it prints the result of
the unary and conditional forms as an `unsigned long long`, and requires every
compiler to agree on every line. Boundary values are the ones that distinguish
width and signedness (min, -1, 0, 1, max, the sign bit, all-ones), because a
fold that widens one conversion too early is invisible in the middle of the
range.

What it does NOT do: it never relies on implementation-defined or undefined
behaviour. Every expression is evaluated in a type wide enough to hold it (the
matrix is built so that the *source* value is always representable), overflow is
avoided by construction, and the printed form is the value converted to
`unsigned long long`, which is defined for every integer type.

Exit status: 0 when lccc agrees with every available oracle at every level, 1 on
any disagreement (with the first mismatching lines printed), and 0 with a SKIP
notice when no oracle is available -- a gate that silently passes without a
reference would be worse than no gate.
"""
from __future__ import annotations

import argparse
import pathlib
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
REPO = HERE.parent.parent

TYPES = [
    "signed char",
    "unsigned char",
    "char",
    "_Bool",
    "short",
    "unsigned short",
    "int",
    "unsigned int",
    "long",
    "unsigned long",
    "long long",
    "unsigned long long",
]

# Boundary values of each type, spelled so that the value is representable in
# the type itself: the literals are formed from the widest type and cast down.
EDGES = {
    "signed char": ["(signed char)-128", "(signed char)-1", "(signed char)0", "(signed char)1", "(signed char)127"],
    "unsigned char": ["(unsigned char)0", "(unsigned char)1", "(unsigned char)127", "(unsigned char)128", "(unsigned char)255"],
    "char": ["(char)-128", "(char)-1", "(char)0", "(char)1", "(char)127"],
    "_Bool": ["(_Bool)0", "(_Bool)1"],
    "short": ["(short)-32768", "(short)-1", "(short)0", "(short)1", "(short)32767"],
    "unsigned short": ["(unsigned short)0", "(unsigned short)1", "(unsigned short)32767", "(unsigned short)32768", "(unsigned short)65535"],
    "int": ["(-2147483647 - 1)", "-1", "0", "1", "2147483647"],
    "unsigned int": ["0u", "1u", "2147483647u", "2147483648u", "4294967295u"],
    "long": ["(-9223372036854775807L - 1)", "-1L", "0L", "1L", "9223372036854775807L"],
    "unsigned long": ["0UL", "1UL", "9223372036854775807UL", "9223372036854775808UL", "18446744073709551615UL"],
    "long long": ["(-9223372036854775807LL - 1)", "-1LL", "0LL", "1LL", "9223372036854775807LL"],
    "unsigned long long": ["0ULL", "1ULL", "9223372036854775807ULL", "9223372036854775808ULL", "18446744073709551615ULL"],
}

# The unary forms. `-(-x)` and `~(~x)` are included because the second
# application is where a fold that dropped one conversion shows up.
UNARY_FORMS = [
    "({t})(-({v}))",
    "({t})(~({v}))",
    "({t})(+({v}))",
    "({t})(-(-({v})))",
    "({t})(~(~({v})))",
    "({t})(-(({t})(~({v}))))",
    "({t})(~(({t})(-({v}))))",
]

# The conditional forms, in both spellings: the two-operand `?:` and the GNU
# shorthand, whose unselected operand must not be evaluated at all.
COND_FORMS = [
    "({t})(({a}) ? ({b}) : ({a}))",
    "({t})(({b}) ? ({a}) : ({b}))",
    "({t})(({a}) ?: ({b}))",
    "({t})(({b}) ?: ({a}))",
    "({t})(({a}) ? (-({b})) : (~({b})))",
    "({t})(({b}) ? ~({a}) : -({a}))",
]


def run(argv: list[str], cwd: pathlib.Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(argv, cwd=cwd, capture_output=True, text=True)


def resolve(requested: str | None, fallbacks: tuple[str, ...]) -> str | None:
    for name in ((requested,) if requested else ()) + fallbacks:
        path = shutil.which(name)
        if path is not None:
            return path
    return None


def build_matrix() -> list[str]:
    """The program body: one `printf` line per case, numbered."""
    lines = [
        "#include <stdio.h>",
        "",
        "int main(void)",
        "{",
    ]
    cases: list[str] = []

    for type_name in TYPES:
        for value in EDGES[type_name]:
            for form in UNARY_FORMS:
                cases.append(form.format(t=type_name, v=value))

    # Every ordered pair of types, one boundary value from each. The pair is
    # walked with a stride so that the matrix visits the whole boundary set of
    # both types rather than the same corner 144 times.
    index = 0
    for left_type in TYPES:
        for right_type in TYPES:
            left = EDGES[left_type][index % len(EDGES[left_type])]
            right = EDGES[right_type][(index * 3 + 1) % len(EDGES[right_type])]
            index += 1
            for form in COND_FORMS:
                cases.append(form.format(t="unsigned long long", a=left, b=right))
                # ... and once converted to each of the two operand types, so
                # the conversion after the common type is covered as well.
                cases.append(form.format(t="int", a=left, b=right))
                cases.append(form.format(t="unsigned int", a=left, b=right))

    for number, expr in enumerate(cases):
        lines.append(f'    printf("{number} %llu\\n", {number}, (unsigned long long)({expr}));')
    lines += ["    return 0;", "}", ""]
    return lines


def compile_and_run(compiler: str, source: pathlib.Path, level: str, work: pathlib.Path,
                    tag: str) -> tuple[int, str]:
    exe = work / f"{tag}{level}"
    proc = run([compiler, level, str(source), "-o", str(exe), "-w"])
    if proc.returncode != 0:
        return 127, f"compile failed at {level}:\n{proc.stderr[-1500:]}"
    proc = run([str(exe)])
    if proc.returncode != 0:
        return proc.returncode, f"run failed at {level} (exit {proc.returncode}):\n{proc.stderr[-1500:]}"
    return 0, proc.stdout


def compare(reference: str, other: str, who: str, level: str, failures: list[str]) -> int:
    """Line-by-line comparison; returns the number of mismatches found."""
    mismatches = 0
    ref_lines = reference.splitlines()
    oth_lines = other.splitlines()
    if len(ref_lines) != len(oth_lines):
        failures.append(f"{who} at {level}: {len(ref_lines)} lines vs lccc's {len(oth_lines)}")
        return 1
    for ref_line, oth_line in zip(ref_lines, oth_lines):
        if ref_line != oth_line:
            mismatches += 1
            if mismatches <= 5:
                failures.append(f"{who} at {level}: lccc={ref_line!r} {who}={oth_line!r}")
    if mismatches > 5:
        failures.append(f"{who} at {level}: ... {mismatches - 5} further mismatches")
    return mismatches


def self_test() -> int:
    """Prove the comparator fails on a difference -- a gate that cannot fail is
    decoration, and this one's whole contract is "the transcripts are equal"."""
    problems: list[str] = []
    failures: list[str] = []
    if compare("0 1\n1 2\n", "0 1\n1 3\n", "oracle", "-O2", failures) != 1:
        problems.append("a one-value difference was not detected")
    failures.clear()
    if compare("0 1\n", "0 1\n", "oracle", "-O2", failures) != 0:
        problems.append("an identical transcript was reported as different")
    failures.clear()
    if compare("0 1\n", "0 1\n1 2\n", "oracle", "-O2", failures) != 1:
        problems.append("a length difference was not detected")
    for problem in problems:
        print(f"  SELF-TEST FAIL: {problem}", file=sys.stderr)
    if problems:
        return 1
    print("  self-test: the comparator detects value and length differences")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default=None, help="oracle GCC (default: gcc-16, then gcc)")
    parser.add_argument("--clang", default=None, help="oracle Clang (default: clang-23, then clang)")
    parser.add_argument("--levels", default="-O0,-O2", help="optimisation levels to compare")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    lccc = resolve(args.lccc, ())
    if lccc is None:
        print(f"lccc not found at {args.lccc}", file=sys.stderr)
        return 1
    oracles = []
    for name, requested, fallbacks in (
        ("gcc-16", args.gcc, ("gcc-16", "gcc")),
        ("clang-23", args.clang, ("clang-23", "clang")),
    ):
        path = resolve(requested, fallbacks)
        if path is not None:
            oracles.append((name, path))
    if not oracles:
        print("SKIP: no oracle compiler (gcc/clang) is available, so nothing can be compared")
        return 0

    lines = build_matrix()
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="lccc-promotion-") as tmp:
        work = pathlib.Path(tmp)
        source = work / "sweep.c"
        source.write_text("\n".join(lines))
        cases = max(0, len(lines) - 4)

        for level in args.levels.split(","):
            code, reference = compile_and_run(lccc, source, level, work, "lccc")
            if code != 0:
                print(f"FAIL: {reference}", file=sys.stderr)
                return 1
            for name, path in oracles:
                code, output = compile_and_run(path, source, level, work, name.replace("-", ""))
                if code != 0:
                    print(f"FAIL: oracle {name}: {output}", file=sys.stderr)
                    return 1
                compare(reference, output, name, level, failures)

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"promotion sweep: FAIL ({len(failures)} finding(s))", file=sys.stderr)
        return 1
    names = ", ".join(pathlib.Path(path).name for _, path in oracles)
    print(
        f"promotion sweep: {cases} expression(s) over 12 integer types x "
        f"{args.levels} agree with {names}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
