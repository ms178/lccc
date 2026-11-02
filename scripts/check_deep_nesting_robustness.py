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
     the single BLOCK budget error (statement expressions parse as
     compound statements, so they fall under the block-class budget;
     the historical shape used to infinite-loop);
  6. block, brace-initializer and nested-record crash classes degrade to
     their own single clean budget diagnostic (generalized frame
     budgets: statements/blocks, initializer lists, record
     definitions) -- each used to abort with a stack overflow on valid
     C input;
  7. hostile C23 #embed parameters (u64::MAX limit, 1 TiB limit, u64::MAX
     offset) never crash the compiler: the resource read is clamped to
     the real file size BEFORE allocating, so these inputs compile
     cleanly on 64-bit hosts (or degrade to a clean "invalid embed
     parameter" error where u64 does not fit in usize).

Budget values are calibrated against measured stack ceilings and
post-parse phase costs on the -O1 fastbuild; see the PARSER_FRAME_BUDGET /
BLOCK_FRAME_BUDGET / TYPE_FRAME_BUDGET comments in
src/frontend/parser/parse.rs.

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

def gen_braces(depth: int) -> str:
    """Deeply nested compound statements (valid C)."""
    return "int main(void) " + "{ " * depth + "return 0;" + " }" * depth + "\n"


def gen_init_braces(depth: int) -> str:
    """Deeply nested brace-initializer lists (valid C)."""
    return ("int a = " + "{ " * depth + "1" + " }" * depth
            + ";\nint main(void) { return a - 1; }\n")


def gen_record_nesting(depth: int, named: bool) -> str:
    """Deeply nested struct DEFINITIONS (valid C), named or anonymous."""
    body = "int x;"
    for i in range(depth):
        if named:
            body = "struct t%d { %s s; } s;" % (i, body)
        else:
            body = "struct { %s s; } s;" % body
    if named:
        return "typedef struct { %s } top;\nint main(void) { return 0; }\n" % body
    return "struct outer { %s } v;\nint main(void) { return 0; }\n" % body


def evaluate_budget_run(rc: int, err: bytes, msg: str = BUDGET_MSG) -> tuple[bool, str]:
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
    count = err.count(msg.encode())
    if count == 0:
        return False, f"rc=1 but no '{msg}' diagnostic"
    if count > 1:
        return False, f"{count} budget diagnostics (exactly one required)"
    if len(err) > STDERR_CAP_BYTES:
        return False, f"diagnostic flood ({len(err)} bytes > {STDERR_CAP_BYTES})"
    return True, f"clean budget error (rc=1, {len(err)} B stderr)"



# --------------------------------------------------------------------------
# C23 #embed I/O crash class (PR #739 audit item F1).  The limit/offset
# parameters are parsed as u64 and used to size the read buffer; without a
# clamp against the real file size, `limit(18446744073709551615)` turns one
# source line into a usize::MAX allocation (allocator abort rc=134 or a
# capacity-overflow panic rc=101).  read_embed_bounded clamps to the
# resource size before allocating, so the adversarial values are
# neutralized; this gate pins that from the outside.
# --------------------------------------------------------------------------

EMBED_RESOURCE = bytes(range(8))  # deterministic 8-byte resource


def gen_embed_limit(limit: int) -> str:
    return ('unsigned char d[] = {#embed "res.bin" limit(%d)};\n'
            'int main(void) { return d[0]; }\n' % limit)


def gen_embed_offset_limit() -> str:
    return ('unsigned char d[] = {#embed "res.bin" '
            'clang::offset(18446744073709551615) limit(1)};\n'
            'int main(void) { return 0; }\n')


def evaluate_embed_adversarial(rc: int, err: bytes) -> tuple[bool, str]:
    """Pure acceptance test for a hostile #embed parameter compile.

    Two legitimate outcomes, by host width:
    * rc == 0 -- u64 fits in usize (64-bit): the limit/offset was
      neutralized by clamping the read to the resource size, the TU
      compiles cleanly;
    * rc == 1 with the documented "invalid embed parameter" diagnostic --
      u64 does not fit in usize (32-bit): the value is rejected outright.
    Anything else -- signals, Rust panic/abort codes (101/134), ICE text,
    or a diagnostic flood -- is a failure.
    """
    if rc < 0:
        return False, f"compiler killed by signal {-rc} (allocation not clamped)"
    if rc in (101, 134):
        return False, (f"rc={rc} -- Rust panic/abort: a hostile limit survived "
                       "the size clamp")
    if len(err) > STDERR_CAP_BYTES:
        return False, f"diagnostic flood ({len(err)} bytes)"
    low = err.lower()
    for ice in (b"internal error", b"panicked", b"capacity overflow"):
        if ice in low:
            return False, f"ICE text ({ice.decode()!r}) in diagnostics"
    if rc == 0:
        return True, "adversarial value neutralized by the size clamp"
    if rc == 1 and b"invalid embed parameter" in err:
        return True, "adversarial value rejected (narrow host)"
    return False, f"rc={rc} with unexpected diagnostics: {err[:200]!r}"


def check_embed_adversarial(name: str, lccc: str, src_text: str, tmp: Path,
                            timeout: int) -> bool:
    src = tmp / f"{name}.c"
    src.write_text(src_text)
    # Quoted-form resources resolve relative to the including file.
    (tmp / "res.bin").write_bytes(EMBED_RESOURCE)
    try:
        rc, err = run_lccc(lccc, src, tmp / f"{name}.s", timeout)
    except subprocess.TimeoutExpired:
        print(f"  FAIL {name}: compiler hung (> {timeout}s)")
        return False
    ok, reason = evaluate_embed_adversarial(rc, err)
    print(f"  {'ok  ' if ok else 'FAIL'} {name}: {reason}")
    return ok


def run_lccc(lccc: str, src: Path, out: Path, timeout: int) -> tuple[int, bytes]:
    proc = subprocess.run(
        [lccc, "-O0", "-S", "-o", str(out), str(src)],
        capture_output=True,
        timeout=timeout,
    )
    return proc.returncode, proc.stderr


def check_clean_budget_error(name: str, lccc: str, src_text: str, tmp: Path,
                             timeout: int, msg: str = BUDGET_MSG) -> bool:
    src = tmp / f"{name}.c"
    src.write_text(src_text)
    try:
        rc, err = run_lccc(lccc, src, tmp / f"{name}.s", timeout)
    except subprocess.TimeoutExpired:
        print(f"  FAIL {name}: compiler hung (> {timeout}s)")
        return False
    ok, reason = evaluate_budget_run(rc, err, msg)
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
    # New crash-class fixture shapes.
    assert gen_braces(2).count("{") == 2 and gen_braces(2).count("}") == 2
    # two initializer braces + one main-body brace
    assert gen_init_braces(2).count("{") == 3
    assert "struct t0" in gen_record_nesting(1, True)
    assert "struct t" not in gen_record_nesting(1, False)
    # The evaluator honors the per-class message.
    blk = b"error: block nesting too deep (exceeds 8192 parser frames)\n"
    assert evaluate_budget_run(1, blk,
                               "block nesting too deep (exceeds 8192 parser frames)")[0]
    assert evaluate_budget_run(1, blk)[0] is False, "wrong message must fail"

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

    # Embed crash-class acceptance logic: neutralized (rc=0) and
    # narrow-host rejection (rc=1 + documented diagnostic) pass; every
    # abnormal outcome fails closed.
    assert evaluate_embed_adversarial(0, b"")[0] is True
    rej = b"error: invalid embed parameter 'limit' value '...'\n"
    assert evaluate_embed_adversarial(1, rej)[0] is True
    assert evaluate_embed_adversarial(134, b"")[0] is False, "abort must fail"
    assert evaluate_embed_adversarial(101, b"")[0] is False, "panic must fail"
    assert "signal 9" in evaluate_embed_adversarial(-9, b"")[1], "signal must fail"
    assert "ICE text" in evaluate_embed_adversarial(1, b"internal error: x")[1]
    assert evaluate_embed_adversarial(0, b"x" * (STDERR_CAP_BYTES + 1))[0] is False
    assert evaluate_embed_adversarial(1, b"some other error\n")[0] is False
    # Fixture shapes: the hostile values appear verbatim.
    assert "limit(18446744073709551615)" in gen_embed_limit(18446744073709551615)
    assert "clang::offset(18446744073709551615)" in gen_embed_offset_limit()
    assert len(EMBED_RESOURCE) == 8

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
        # 1b. statement-expression crash class -> termination guarantee.
        # Statement expressions ARE compound statements, so the block-class
        # budget (not the expression budget) catches them.
        ok &= check_clean_budget_error("stmtexpr_11000", lccc, gen_stmtexpr(11000),
                                       tmp, args.timeout,
                                       "block nesting too deep (exceeds 8192 parser frames)")
        # 1c. block / initializer / record nesting crash classes (F5):
        # generalized frame budgets must degrade to their own clean
        # single diagnostic instead of a stack-overflow abort.
        ok &= check_clean_budget_error("block_braces_200k", lccc,
                                       gen_braces(200_000), tmp, args.timeout,
                                       "block nesting too deep (exceeds 8192 parser frames)")
        ok &= check_clean_budget_error("init_braces_200k", lccc,
                                       gen_init_braces(200_000), tmp, args.timeout,
                                       "initializer nesting too deep (exceeds 32768 parser frames)")
        ok &= check_clean_budget_error("record_named_50k", lccc,
                                       gen_record_nesting(50_000, True), tmp,
                                       args.timeout,
                                       "type nesting too deep (exceeds 2048 parser frames)")
        ok &= check_clean_budget_error("record_anon_50k", lccc,
                                       gen_record_nesting(50_000, False), tmp,
                                       args.timeout,
                                       "type nesting too deep (exceeds 2048 parser frames)")
        # 1d. #embed I/O crash class -> size clamp neutralizes hostile limits
        ok &= check_embed_adversarial("embed_limit_u64max", lccc,
                                      gen_embed_limit(18446744073709551615),
                                      tmp, args.timeout)
        ok &= check_embed_adversarial("embed_limit_1tib", lccc,
                                      gen_embed_limit(1099511627776),
                                      tmp, args.timeout)
        ok &= check_embed_adversarial("embed_offset_u64max_limit_1", lccc,
                                      gen_embed_offset_limit(), tmp, args.timeout)
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
