#!/usr/bin/env python3
"""Deep-nesting robustness gate (EDG transplant item E5).

Adversarially nested C expressions used to overflow the compiler's stack and
abort (rc=134): the recursive-descent expression parser re-enters the whole
precedence climb once per parenthesis level, and each of those frames carries
`Expr`-sized by-value temporaries plus the large locals of the primary/cast
functions.  Measured on the -O1 fastbuild, ~4800 nested-parenthesis levels
overflowed even the 64 MB compiler thread.  GCC 14 segfaults its cc1 on the
same input, so this class is a genuine robustness differentiator.

The parser now counts live expression-parse frames against a budget
(`Parser::EXPR_FRAME_BUDGET`) and degrades to a single clean
"expression nesting too deep" error instead of aborting; a Clang-style cap
on rendered diagnostics keeps the failure output bounded.

This gate pins the whole contract:

  1. crash-class inputs (5000 paren levels) exit 1 with exactly the budget
     diagnostic -- never an abort/segv, never a diagnostic flood (>256 KB);
  2. deep-but-legal shapes that have always compiled keep compiling
     (right-nested ternary, unary chains) -- the budget must not reject
     inputs that were safe before;
  3. an in-budget nested expression still evaluates CORRECTLY end to end
     (compile -> assemble -> run -> value check), so the budget path cannot
     silently truncate real code;
  4. the preprocessor's #if evaluator survives a flat 100k-term expression.

The fixtures are generated at test time (they are trivial and reproducible;
committing 50 KB of repeated parentheses would be dead weight).

Usage:
  scripts/check_deep_nesting_robustness.py [--lccc target/fastbuild/lccc]
                                           [--cc gcc]
  scripts/check_deep_nesting_robustness.py --selftest   # no compiler needed
"""
from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

BUDGET_MSG = "expression nesting too deep"
STDERR_CAP_BYTES = 256 * 1024
CRASH_CODES = {134, 137, 139}  # abort / OOM-kill / segv


# --------------------------------------------------------------------------
# Fixture generators.  Pure functions of the depth so the gate is
# reproducible; --selftest validates their invariants.
# --------------------------------------------------------------------------

def gen_paren_add(depth: int) -> str:
    """The measured crash class: `((((1) + 0) + 0) ... )`."""
    e = "1"
    for _ in range(depth):
        e = f"({e}) + 0"
    return f"int x = {e};\n"


def gen_paren_pure(depth: int) -> str:
    """Pure parenthesis nesting: `((((1))))`."""
    e = "1"
    for _ in range(depth):
        e = f"({e})"
    return f"int x = {e};\n"


def gen_ternary(depth: int) -> str:
    """Right-nested conditional.  Cheap on the stack (the binary-operator
    chain unwinds before the then-arm recurses): compiled long before the
    budget existed and must keep compiling."""
    e = "1"
    for _ in range(depth):
        e = f"1 ? {e} : 2"
    return f"int x = {e};\n"


def gen_unary(depth: int) -> str:
    """`- - - - ... 1` — unary/cast self-recursion."""
    e = "1"
    for _ in range(depth):
        e = f"-{e}"
    return f"int x = {e};\n"


def gen_semantic(depth: int) -> str:
    """`((((1) + 1) + 1) ... )` at `depth` levels evaluates to depth + 1.
    Single parenthesis per level, so depth 1000 stays far inside the budget
    and exercises the full pipeline (sema, const-eval, lowering) on a deep
    AST."""
    e = "1"
    for _ in range(depth):
        e = f"({e}) + 1"
    return (
        "#include <stdio.h>\n"
        f"int x = {e};\n"
        'int main(void){printf("%d\\n", x);return 0;}\n'
    )


def gen_if_flat(terms: int) -> str:
    """Flat (non-nested) #if expression: must not be affected at all."""
    return f"#if ({'1' + ' + 1' * terms})\nint ok = 1;\n#endif\n"


# --------------------------------------------------------------------------
# Check machinery
# --------------------------------------------------------------------------

def run_lccc(lccc: str, src: Path, out: Path, timeout: int) -> tuple[int, bytes]:
    proc = subprocess.run(
        [lccc, "-O0", "-S", "-o", str(out), str(src)],
        capture_output=True,
        timeout=timeout,
    )
    return proc.returncode, proc.stderr


def check_clean_budget_error(name: str, lccc: str, src_text: str, tmp: Path,
                             timeout: int) -> bool:
    src = tmp / f"{name}.c"
    src.write_text(src_text)
    try:
        rc, err = run_lccc(lccc, src, tmp / f"{name}.s", timeout)
    except subprocess.TimeoutExpired:
        print(f"  FAIL {name}: compiler hung (> {timeout}s)")
        return False
    if rc in CRASH_CODES:
        print(f"  FAIL {name}: crashed with rc={rc} (budget must degrade to an error)")
        return False
    if rc == 0:
        print(f"  FAIL {name}: accepted input beyond the nesting budget")
        return False
    if BUDGET_MSG.encode() not in err:
        print(f"  FAIL {name}: rc={rc} but no '{BUDGET_MSG}' diagnostic")
        return False
    if len(err) > STDERR_CAP_BYTES:
        print(f"  FAIL {name}: diagnostic flood ({len(err)} bytes > {STDERR_CAP_BYTES})")
        return False
    print(f"  ok   {name}: clean budget error (rc=1, {len(err)} B stderr)")
    return True


def check_compiles(name: str, lccc: str, src_text: str, tmp: Path,
                   timeout: int) -> bool:
    src = tmp / f"{name}.c"
    src.write_text(src_text)
    try:
        rc, err = run_lccc(lccc, src, tmp / f"{name}.s", timeout)
    except subprocess.TimeoutExpired:
        print(f"  FAIL {name}: compiler hung (> {timeout}s)")
        return False
    if rc != 0:
        first = err.decode(errors="replace").splitlines()[:2]
        print(f"  FAIL {name}: rc={rc} on a deep-but-legal input: {first}")
        return False
    print(f"  ok   {name}: deep-but-legal input still compiles")
    return True


def check_semantic(name: str, lccc: str, cc: str, depth: int, tmp: Path,
                   timeout: int) -> bool:
    src = tmp / f"{name}.c"
    src.write_text(gen_semantic(depth))
    asm = tmp / f"{name}.s"
    try:
        rc, err = run_lccc(lccc, src, asm, timeout)
    except subprocess.TimeoutExpired:
        print(f"  FAIL {name}: compiler hung (> {timeout}s)")
        return False
    if rc != 0:
        print(f"  FAIL {name}: in-budget nesting (depth {depth}) rejected: rc={rc}")
        return False
    binary = tmp / name
    link = subprocess.run([cc, str(asm), "-o", str(binary)], capture_output=True,
                          timeout=timeout)
    if link.returncode != 0:
        print(f"  FAIL {name}: host assembler/linker rejected lccc output: "
              f"{link.stderr.decode(errors='replace').splitlines()[:2]}")
        return False
    run = subprocess.run([str(binary)], capture_output=True, timeout=timeout)
    want = f"{depth + 1}\n"
    got = run.stdout.decode(errors="replace")
    if run.returncode != 0 or got != want:
        print(f"  FAIL {name}: wrong value: want {want!r} got {got!r} rc={run.returncode}")
        return False
    print(f"  ok   {name}: depth-{depth} nested initializer evaluates to {depth + 1}")
    return True


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--cc", default=os.environ.get("CC", "gcc"))
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--selftest", action="store_true",
                    help="validate fixture invariants; no compiler needed")
    args = ap.parse_args()

    if args.selftest:
        # Fixture invariants: the generators must produce exactly the shape
        # the budget analysis assumes (nesting counts, flatness).
        assert gen_paren_add(3) == "int x = (((1) + 0) + 0) + 0;\n"
        assert gen_paren_pure(3) == "int x = (((1)));\n"
        assert gen_ternary(2) == "int x = 1 ? 1 ? 1 : 2 : 2;\n"
        assert gen_unary(3) == "int x = ---1;\n"
        assert gen_if_flat(3) == "#if (1 + 1 + 1 + 1)\nint ok = 1;\n#endif\n"
        sem = gen_semantic(2)
        assert "int x = ((1) + 1) + 1;" in sem and 'printf("%d' in sem
        # Depth bookkeeping: paren shapes grow by one level per iteration.
        assert gen_paren_pure(500).count("(") == 500
        assert gen_paren_add(500).count("+ 0") == 500
        # The crash-class depth is far beyond the ~1300 measured ceiling.
        assert gen_paren_pure(5000).count("(") == 5000
        print("deep-nesting selftest: PASS")
        return 0

    lccc = args.lccc
    if not Path(lccc).exists():
        print(f"FATAL: compiler not found: {lccc} (build scripts/build_lccc_fast.sh)")
        return 1

    print("== deep-nesting robustness gate (E5: parser frame budget) ==")
    ok = True
    with tempfile.TemporaryDirectory(prefix="lccc_deepnest_") as td:
        tmp = Path(td)
        # 1. crash class -> clean bounded error, never an abort
        ok &= check_clean_budget_error("paren_add_5000", lccc, gen_paren_add(5000),
                                       tmp, args.timeout)
        ok &= check_clean_budget_error("paren_pure_5000", lccc, gen_paren_pure(5000),
                                       tmp, args.timeout)
        # 2. deep-but-legal shapes must keep compiling
        ok &= check_compiles("ternary_8000", lccc, gen_ternary(8000), tmp,
                             args.timeout)
        ok &= check_compiles("unary_5000", lccc, gen_unary(5000), tmp, args.timeout)
        # 3. in-budget nesting evaluates correctly end to end
        ok &= check_semantic("semantic_1000", lccc, args.cc, 1000, tmp,
                             args.timeout)
        # 4. flat #if expressions are untouched
        ok &= check_compiles("if_flat_100k", lccc, gen_if_flat(100_000), tmp,
                             args.timeout)

    print("deep-nesting robustness: " + ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
