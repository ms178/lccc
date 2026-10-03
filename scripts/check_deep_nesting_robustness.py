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
on rendered diagnostics keeps the failure output bounded; statement/block
recovery loops carry termination guarantees so budget exhaustion cannot
spin forever.

This gate pins the whole contract:

  1. crash-class inputs (5000 paren levels) exit with EXACTLY the budget
     diagnostic and rc=1 -- never an abort/segv/signal (checked
     explicitly), never a hang (hard timeout), never a diagnostic flood
     (>256 KB), never zero or duplicate budget diagnostics;
  2. deep-but-legal shapes that have always compiled keep compiling
     (right-nested ternary, unary-negation chains) -- the budget must not
     reject inputs that were safe before;
  3. an in-budget nested expression still evaluates CORRECTLY end to end
     (compile -> assemble -> run -> value check), so the budget path cannot
     silently truncate real code;
  4. the preprocessor's #if evaluator survives a flat 100k-term expression;
  5. a GNU statement-expression nesting beyond the budget terminates with
     the same single budget error (the historical infinite-loop shape).

The fixtures are generated at test time (they are trivial and reproducible;
committing 50 KB of repeated parentheses would be dead weight).

The pass/fail decision for the crash class is factored into
`evaluate_budget_run`, a pure function of (returncode, stderr) that is
unit-tested by --selftest against success, zero-exit, arbitrary-failure,
signal-termination, missing-diagnostic and duplicate-diagnostic outcomes.

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
    """`- - - - ... 1` — unary/cast self-recursion.

    Whitespace-separated negations: maximal-munch lexing would fold the
    unspaced form `---1` into decrement operators (`--` `-`), which is a
    different parse shape; the spaced form is unambiguously a chain of
    `depth` unary negations for every depth (odd depths negate, even
    depths cancel)."""
    return f"int x = {'- ' * depth}1;\n"


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


def gen_stmtexpr(depth: int) -> str:
    """GNU statement-expression nesting, `({ ({ ... 1 ... }) })`, one
    `({ ... })` per level.  Beyond the frame budget this shape used to
    spin a compound-statement recovery loop forever; it must now terminate
    with the single budget diagnostic."""
    return "int x = " + "({ " * depth + "1" + " })" * depth + ";\n"


# --------------------------------------------------------------------------
# Check machinery
# --------------------------------------------------------------------------

def evaluate_budget_run(rc: int, err: bytes) -> tuple[bool, str]:
    """Pure acceptance test for a crash-class compile.

    Returns (ok, reason).  The contract the budget feature promises:

    * rc == 1 — the documented error exit.  Zero means the input was
      accepted beyond the budget; any other positive code or a negative
      code (Python reports signal-killed children as -SIGNUM, e.g. -11
      for SIGSEGV) means a crash or some other abnormal end, NOT the
      clean degradation this gate exists to pin.
    * exactly one budget diagnostic — zero means a silent accept path,
      more than one means the flood guard failed.
    * bounded stderr — the diagnostic flood guard.
    """
    if rc < 0:
        return False, f"compiler killed by signal {-rc} (must degrade to an error)"
    if rc == 0:
        return False, "accepted input beyond the nesting budget (rc=0)"
    if rc != 1:
        return False, f"rc={rc} but the documented error exit is 1"
    count = err.count(BUDGET_MSG.encode())
    if count == 0:
        return False, f"rc=1 but no '{BUDGET_MSG}' diagnostic"
    if count > 1:
        return False, f"{count} budget diagnostics (exactly one required)"
    if len(err) > STDERR_CAP_BYTES:
        return False, f"diagnostic flood ({len(err)} bytes > {STDERR_CAP_BYTES})"
    return True, f"clean budget error (rc=1, {len(err)} B stderr)"


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
    ok, reason = evaluate_budget_run(rc, err)
    print(f"  {'ok  ' if ok else 'FAIL'} {name}: {reason}")
    return ok


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


def selftest() -> int:
    # Fixture invariants: the generators must produce exactly the shape
    # the budget analysis assumes (nesting counts, flatness).
    assert gen_paren_add(3) == "int x = (((1) + 0) + 0) + 0;\n"
    assert gen_paren_pure(3) == "int x = (((1)));\n"
    assert gen_ternary(2) == "int x = 1 ? 1 ? 1 : 2 : 2;\n"
    # Whitespace-separated negations (maximal munch cannot fold `- -`).
    assert gen_unary(3) == "int x = - - - 1;\n"
    assert gen_unary(4) == "int x = - - - - 1;\n"
    assert gen_unary(3).count("- ") == 3 and gen_unary(4).count("- ") == 4
    assert gen_if_flat(3) == "#if (1 + 1 + 1 + 1)\nint ok = 1;\n#endif\n"
    assert gen_stmtexpr(2) == "int x = ({ ({ 1 }) });\n"
    sem = gen_semantic(2)
    assert "int x = ((1) + 1) + 1;" in sem and 'printf("%d' in sem
    # Depth bookkeeping: paren shapes grow by one level per iteration.
    assert gen_paren_pure(500).count("(") == 500
    assert gen_paren_add(500).count("+ 0") == 500
    # The crash-class depth is far beyond the ~1300 measured ceiling.
    assert gen_paren_pure(5000).count("(") == 5000

    # Acceptance-logic unit tests: the gate must fail closed on every
    # abnormal outcome and pass only the documented clean degradation.
    good = ("...error: " + BUDGET_MSG + " (exceeds 32768 parser frames)\n").encode()
    assert evaluate_budget_run(1, good) == (
        True, f"clean budget error (rc=1, {len(good)} B stderr)")
    assert evaluate_budget_run(0, good)[0] is False, "rc=0 must fail"
    assert evaluate_budget_run(2, good)[0] is False, "arbitrary rc must fail"
    assert evaluate_budget_run(134, good)[0] is False, "abort must fail"
    assert "signal 11" in evaluate_budget_run(-11, good)[1], "SIGSEGV must fail"
    assert evaluate_budget_run(-6, good)[0] is False, "SIGABRT must fail"
    assert evaluate_budget_run(-9, good)[0] is False, "SIGKILL must fail"
    assert "no " + "'" + BUDGET_MSG + "'" in evaluate_budget_run(1, b"other\n")[1]
    assert "2 budget diagnostics" in evaluate_budget_run(1, good + good)[1]
    assert "flood" in evaluate_budget_run(1, good + b"x" * (STDERR_CAP_BYTES + 1))[1]

    print("deep-nesting selftest: PASS")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--lccc", default="target/fastbuild/lccc")
    ap.add_argument("--cc", default=os.environ.get("CC", "gcc"))
    ap.add_argument("--timeout", type=int, default=60)
    ap.add_argument("--selftest", action="store_true",
                    help="validate fixture invariants; no compiler needed")
    args = ap.parse_args()

    if args.selftest:
        return selftest()

    lccc = args.lccc
    if not Path(lccc).exists():
        print(f"FATAL: compiler not found: {lccc} (build scripts/build_lccc_fast.sh)")
        return 1

    print("== deep-nesting robustness gate (E5: parser frame budget) ==")
    ok = True
    with tempfile.TemporaryDirectory(prefix="lccc_deepnest_") as td:
        tmp = Path(td)
        # 1. crash class -> clean bounded error, never an abort/hang
        ok &= check_clean_budget_error("paren_add_5000", lccc, gen_paren_add(5000),
                                       tmp, args.timeout)
        ok &= check_clean_budget_error("paren_pure_5000", lccc, gen_paren_pure(5000),
                                       tmp, args.timeout)
        # 1b. statement-expression crash class -> termination guarantee
        ok &= check_clean_budget_error("stmtexpr_11000", lccc, gen_stmtexpr(11000),
                                       tmp, args.timeout)
        # 2. deep-but-legal shapes must keep compiling
        ok &= check_compiles("ternary_8000", lccc, gen_ternary(8000), tmp,
                             args.timeout)
        ok &= check_compiles("unary_5000", lccc, gen_unary(5000), tmp,
                             args.timeout)
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
