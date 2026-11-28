#!/usr/bin/env python3
"""Which operands does each operator take? lccc's answer against GCC 16.2's.

Why
---
An operator has constraints on its operands (C11 6.5.3.3, 6.5.5 to 6.5.14):
`~` takes an integer, `%` two, `<` no complex value, `-` no two pointers to
different things. A compiler that does not check one of them does not crash
on the program that violates it: it compiles something. lccc compiled `~d`
of a `double` as the complement of its bits, `-p` of a pointer as arithmetic
on the address, and, where an operand was the result of a builtin, handed a
floating operand of `&` to the code generator, which stopped with an
internal error. Each of these was found by somebody's test of something
else. This tool asks the question itself, for every operator and every kind
of operand, and GCC answers it.

How
---
One translation unit per cell: an object of each operand kind and one
statement that applies the operator. Both compilers run `-fsyntax-only -w`;
a cell is the first error each gives, or "ok". The unary table is 20 kinds
by 10 operators, the binary table 28 kinds by 28 by 18.

A cell is one of
  same      both accept, or both refuse (whatever the words)
  wording   both refuse, and the first errors differ
  accepts   lccc accepts what GCC refuses
  refuses   lccc refuses what GCC accepts: a valid program does not compile
  crash     lccc stops with an internal error

`--check` fails on a unary cell that differs and is not recorded open
below (or differs in another way than recorded), on any binary `refuses` or
`crash` cell, and on more binary `accepts` cells than recorded. The record
only goes down: a rule that closes cells lowers it in the same commit.

    scripts/operand_constraint_matrix.py --lccc target/fastbuild/lccc --check
    scripts/operand_constraint_matrix.py --lccc BIN --table unary -v
"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

# kind -> (declaration, expression). A declaration is at file scope, so that
# a function is a function and an array an array.
UNARY_KINDS = {
    "int": "int x", "bool": "_Bool x", "enum": "enum E { A } x", "float": "float x",
    "double": "double x", "ldouble": "long double x", "f16": "_Float16 x",
    "f128": "__float128 x", "dec64": "_Decimal64 x", "complex": "_Complex double x",
    "icomplex": "_Complex int x", "pointer": "int *x", "voidptr": "void *x",
    "array": "int x[3]", "function": "void x(void)", "record": "struct S { int a; } x",
    "union": "union U { int a; } x",
    "ivec": "__attribute__((vector_size(16))) int x",
    "fvec": "__attribute__((vector_size(16))) float x", "bitint": "_BitInt(7) x",
}
UNARY_OPERATORS = {
    "+": "(void)+x;", "-": "(void)-x;", "~": "(void)~x;", "!": "(void)!x;",
    "__real__": "(void)__real__ x;", "__imag__": "(void)__imag__ x;",
    "++x": "++x;", "--x": "--x;", "x++": "x++;", "x--": "x--;",
}
BINARY_KINDS = {
    "int": ("int vi;", "vi"), "bool": ("_Bool vb;", "vb"), "enum": ("enum E { A } ve;", "ve"),
    "uchar": ("unsigned char vc;", "vc"), "ullong": ("unsigned long long vull;", "vull"),
    "float": ("float vf;", "vf"), "double": ("double vd;", "vd"),
    "ldouble": ("long double vld;", "vld"), "f128": ("__float128 vq;", "vq"),
    "complex": ("_Complex double vz;", "vz"),
    "pointer": ("int *vp;", "vp"), "voidptr": ("void *vv;", "vv"), "charptr": ("char *vs;", "vs"),
    "array": ("int va[3];", "va"), "function": ("void vfn(void);", "vfn"),
    "record": ("struct S { int a; } vr;", "vr"),
    "ivec": ("__attribute__((vector_size(16))) int viv;", "viv"),
    "fvec": ("__attribute__((vector_size(16))) float vfv;", "vfv"),
    "call_int": ("", "__builtin_popcount(1)"), "call_double": ("", "__builtin_sqrt(2.0)"),
    "call_ulong": ("", '__builtin_strlen("a")'), "call_ptr": ("", "__builtin_alloca(4)"),
    "call_float": ("", "__builtin_fabsf(1.0f)"), "user_double": ("double ud(void);", "ud()"),
    "intconst": ("", "3"), "fconst": ("", "2.5"), "null": ("", "((void *)0)"), "zero": ("", "0"),
}
BINARY_OPERATORS = ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>", "==", "!=", "<", "<=",
                    ">", ">=", "&&", "||"]

# Unary cells that differ and are known, each with how: lccc has no
# `_BitInt`, and refuses the declaration.
UNARY_OPEN = {("bitint", operator): "refuses" for operator in UNARY_OPERATORS}
# Binary cells lccc accepts and GCC refuses, by what an operand is. Measured
# 2026-11-28; a count may only fall.
#   builtin  an operand is the result of a builtin the analyzer does not
#            type: one that is no library function (`__builtin_popcount`).
#            A library builtin has the result of GCC's prototype (1494 cells
#            before that), and a floating operand that decides an
#            integer-only operator is judged whatever the builtin
#   vector   a GNU vector against a scalar GCC does not convert to it, or
#            `&&`/`||` of a vector
#   plain    differences of pointers to incompatible types
BINARY_OPEN = {"builtin": 295, "vector": 752, "plain": 22}

CRASH = re.compile(r"panicked at|internal error|internal compiler error|RUST_BACKTRACE")


def unary_source(kind: str, operator: str) -> str:
    declaration, statement = UNARY_KINDS[kind], UNARY_OPERATORS[operator]
    if kind == "function":
        return f"extern {declaration};\nvoid f(void) {{ {statement} }}\n"
    return f"void f(void) {{ {declaration}; {statement} }}\n"


def binary_source(left: str, right: str, operator: str) -> str:
    declarations = "\n".join(sorted({BINARY_KINDS[left][0], BINARY_KINDS[right][0]} - {""}))
    return (f"{declarations}\nvoid f(void) {{ (void)({BINARY_KINDS[left][1]} {operator} "
            f"{BINARY_KINDS[right][1]}); }}\n")


def verdict_of(returncode: int, stderr: str) -> str:
    """"ok", "crash", or the first error."""
    if CRASH.search(stderr):
        return "crash"
    errors = [line.split("error: ", 1)[1] for line in stderr.splitlines() if "error: " in line]
    if errors:
        return errors[0]
    return "ok" if returncode == 0 else "crash"


def classify(gcc: str, lccc: str) -> str:
    if lccc == "crash":
        return "crash"
    if gcc == lccc or (gcc != "ok" and lccc != "ok" and gcc == lccc):
        return "same"
    if gcc == "ok":
        return "refuses"
    if lccc == "ok":
        return "accepts"
    return "wording"


def operand_class(kind: str) -> str:
    if kind.startswith("call_"):
        return "builtin"
    return "vector" if kind in ("ivec", "fvec") else "plain"


def cell_class(left: str, right: str) -> str:
    classes = {operand_class(left), operand_class(right)}
    if "builtin" in classes:
        return "builtin"
    return "vector" if "vector" in classes else "plain"


def run_cells(sources: list[str], compilers: list[list[str]], directory: Path,
              jobs: int) -> list[list[str]]:
    """The verdict of each compiler on each source."""
    def one(index: int) -> list[str]:
        path = directory / f"c{index}.c"
        path.write_text(sources[index])
        verdicts = []
        for compiler in compilers:
            result = subprocess.run([*compiler, "-fsyntax-only", "-w", str(path)],
                                    capture_output=True, text=True, timeout=120)
            verdicts.append(verdict_of(result.returncode, result.stderr))
        path.unlink()
        return verdicts
    with concurrent.futures.ThreadPoolExecutor(max_workers=jobs) as pool:
        return list(pool.map(one, range(len(sources))))


def unary_table(lccc: str, gcc: str, std: str, directory: Path, jobs: int):
    cells = [(kind, operator) for kind in UNARY_KINDS for operator in UNARY_OPERATORS]
    verdicts = run_cells([unary_source(*cell) for cell in cells],
                         [[gcc, f"-std={std}"], [lccc, f"-std={std}"]], directory, jobs)
    return [(cell, theirs, ours, classify(theirs, ours))
            for cell, (theirs, ours) in zip(cells, verdicts)]


def binary_table(lccc: str, gcc: str, std: str, directory: Path, jobs: int):
    cells = [(left, right, operator) for left in BINARY_KINDS for right in BINARY_KINDS
             for operator in BINARY_OPERATORS]
    verdicts = run_cells([binary_source(*cell) for cell in cells],
                         [[gcc, f"-std={std}"], [lccc, f"-std={std}"]], directory, jobs)
    return [(cell, theirs, ours, classify(theirs, ours))
            for cell, (theirs, ours) in zip(cells, verdicts)]


def unary_problems(table) -> list[str]:
    problems = []
    for cell, theirs, ours, kind in table:
        if kind != "same" and UNARY_OPEN.get(cell) != kind:
            problems.append(f"unary {cell[1]} of {cell[0]}: GCC {theirs!r}, lccc {ours!r}")
    closed = sorted(cell for cell, _, _, kind in table if kind == "same" and cell in UNARY_OPEN)
    if closed:
        problems.append(f"unary cells recorded open that agree now (take them out of "
                        f"UNARY_OPEN): {closed}")
    return problems


def binary_counts(table) -> dict[str, int]:
    counts: collections.Counter = collections.Counter()
    for (left, right, _), _, _, kind in table:
        if kind == "accepts":
            counts[cell_class(left, right)] += 1
    return {name: counts.get(name, 0) for name in BINARY_OPEN}


def binary_problems(table) -> list[str]:
    problems = []
    for (left, right, operator), theirs, ours, kind in table:
        if kind in ("refuses", "crash"):
            problems.append(f"binary {left} {operator} {right}: GCC {theirs!r}, lccc {ours!r}")
    for name, count in binary_counts(table).items():
        if count > BINARY_OPEN[name]:
            problems.append(f"binary cells lccc accepts and GCC refuses, {name}: {count}, "
                            f"recorded {BINARY_OPEN[name]}")
        elif count < BINARY_OPEN[name]:
            problems.append(f"binary cells lccc accepts and GCC refuses, {name}: {count}, "
                            f"recorded {BINARY_OPEN[name]} (lower the record in BINARY_OPEN)")
    return problems


def self_test() -> int:
    assert verdict_of(0, "") == "ok"
    assert verdict_of(1, "a.c:1:2: error: wrong type argument to bit-complement\n") == \
        "wrong type argument to bit-complement"
    assert verdict_of(1, "x\na.c:1:2: error: first\na.c:2:2: error: second\n") == "first"
    assert verdict_of(101, "thread 'main' panicked at src/x.rs:1:1:\nboom\n") == "crash"
    assert verdict_of(1, "ccc: internal error: unsupported float binop\n") == "crash"
    # A compiler that dies without a word did not accept the program.
    assert verdict_of(-11, "") == "crash"
    assert classify("ok", "ok") == "same"
    assert classify("e", "e") == "same"
    assert classify("e", "ok") == "accepts"
    assert classify("ok", "e") == "refuses"
    assert classify("e", "other") == "wording"
    assert classify("ok", "crash") == "crash" and classify("e", "crash") == "crash"
    assert cell_class("int", "double") == "plain"
    assert cell_class("ivec", "int") == "vector" and cell_class("fvec", "ivec") == "vector"
    assert cell_class("call_int", "ivec") == "builtin" and cell_class("null", "call_ptr") == "builtin"
    assert len(UNARY_KINDS) * len(UNARY_OPERATORS) == 200
    assert len(BINARY_KINDS) ** 2 * len(BINARY_OPERATORS) == 14112
    assert "extern void x(void);" in unary_source("function", "x++")
    assert unary_source("double", "~") == "void f(void) { double x; (void)~x; }\n"
    assert binary_source("double", "call_int", "&") == \
        "double vd;\nvoid f(void) { (void)(vd & __builtin_popcount(1)); }\n"
    assert binary_source("record", "enum", "+").count("\n") == 3

    same = (("int", "+"), "ok", "ok", "same")
    assert unary_problems([same]) == []
    # A cell recorded open is open in the way recorded and in no other.
    assert unary_problems([(("bitint", "+"), "ok", "e", "refuses")]) == []
    assert unary_problems([(("bitint", "+"), "ok", "crash", "crash")])
    assert unary_problems([(("bitint", "+"), "e", "ok", "accepts")])
    assert unary_problems([(("double", "~"), "e", "ok", "accepts")])
    assert unary_problems([(("double", "~"), "ok", "e", "refuses")])
    assert unary_problems([(("double", "~"), "ok", "crash", "crash")])
    assert "take them out" in unary_problems([(("bitint", "~"), "ok", "ok", "same")])[0]

    def accepts(left, right, count):
        return [((left, right, "+"), "e", "ok", "accepts")] * count
    at_record = (accepts("call_int", "int", BINARY_OPEN["builtin"])
                 + accepts("ivec", "int", BINARY_OPEN["vector"])
                 + accepts("pointer", "voidptr", BINARY_OPEN["plain"]))
    assert binary_problems(at_record) == []
    assert "recorded" in binary_problems(at_record + accepts("ivec", "bool", 1))[0]
    assert "lower the record" in binary_problems(at_record[1:])[0]
    assert binary_problems(at_record + [(("int", "int", "+"), "ok", "e", "refuses")])
    assert binary_problems(at_record + [(("int", "double", "&"), "e", "crash", "crash")])
    print("operand_constraint_matrix: self-test OK")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--lccc", default="target/fastbuild/lccc")
    parser.add_argument("--gcc", default=os.environ.get("GCC", "gcc-16"))
    parser.add_argument("--std", default="gnu17")
    parser.add_argument("--table", choices=("unary", "binary", "both"), default="both")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args(argv)
    if args.self_test:
        return self_test()
    for tool in (args.lccc, args.gcc):
        probe = subprocess.run([tool, "--version"], capture_output=True, text=True)
        if probe.returncode != 0:
            print(f"operand_constraint_matrix: {tool} does not run", file=sys.stderr)
            return 2
    version = subprocess.run([args.gcc, "-dumpfullversion"], capture_output=True, text=True)
    if not version.stdout.startswith("16."):
        print(f"operand_constraint_matrix: {args.gcc} is {version.stdout.strip()}; the record "
              f"is GCC 16.2's", file=sys.stderr)
        return 2
    problems: list[str] = []
    with tempfile.TemporaryDirectory() as tmp:
        directory = Path(tmp)
        if args.table in ("unary", "both"):
            table = unary_table(args.lccc, args.gcc, args.std, directory, args.jobs)
            kinds = collections.Counter(kind for _, _, _, kind in table)
            print(f"unary: {len(table)} cells: {kinds['same']} same, {kinds['accepts']} lccc "
                  f"accepts, {kinds['refuses']} lccc refuses, {kinds['wording']} worded "
                  f"otherwise, {kinds['crash']} crash")
            if args.verbose:
                for cell, theirs, ours, kind in table:
                    if kind != "same":
                        print(f"  {kind:8} {cell[1]:8} {cell[0]:9} GCC: {theirs[:60]:60} "
                              f"lccc: {ours[:60]}")
            problems += unary_problems(table)
        if args.table in ("binary", "both"):
            table = binary_table(args.lccc, args.gcc, args.std, directory, args.jobs)
            kinds = collections.Counter(kind for _, _, _, kind in table)
            counts = binary_counts(table)
            print(f"binary: {len(table)} cells: {kinds['same']} same, {kinds['accepts']} lccc "
                  f"accepts ({', '.join(f'{name} {count}' for name, count in counts.items())}), "
                  f"{kinds['refuses']} lccc refuses, {kinds['wording']} worded otherwise, "
                  f"{kinds['crash']} crash")
            if args.verbose:
                for (left, right, operator), theirs, ours, kind in table:
                    if kind not in ("same", "wording"):
                        print(f"  {kind:8} {left:11} {operator:2} {right:11} GCC: {theirs[:70]}")
            problems += binary_problems(table)
    if args.check and problems:
        for problem in problems[:40]:
            print(f"FAIL: {problem}", file=sys.stderr)
        if len(problems) > 40:
            print(f"... and {len(problems) - 40} more", file=sys.stderr)
        return 1
    if args.check:
        print("operand_constraint_matrix: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
