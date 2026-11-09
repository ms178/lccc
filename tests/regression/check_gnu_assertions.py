#!/usr/bin/env python3
"""Gate GNU preprocessor assertions (#assert / #unassert / -A / #pred(ans)).

Scope: the deprecated-but-shipped GCC extension documented under "Obsolete
Features" in cpp(1), exercised by gcc.c-torture/compile/950919-1.c.  Every
behaviour pinned here was measured against GCC 16.2 (gcc-16) before being
written down; the GCC invocations that produced each pin are reproducible by
running this script with --gcc/--gcc32, which re-derives the expectation from
the live oracle instead of trusting the pin.

Two layers, deliberately:
  * Pinned expectations ALWAYS run.  They need no oracle, so the gate cannot
    silently weaken on a machine without gcc-16, and a regression in LCCC is
    caught even if the oracle itself moves.
  * --gcc PATH (and --gcc32 PATH for the -m32 lane) additionally replays the
    token-stream corpus differentially against the oracle, so a WRONG PIN is
    also caught: the pin must match both LCCC and GCC or the gate is red.

What is checked (each check can fail; --self-test proves it):
  1. Token streams for the full semantic corpus at x86-64 and i686 (-m32):
     predefined system/cpu/machine answers, bare-predicate tests, whitespace
     canonicalization inside answers, first-')' answer termination,
     #assert/#unassert flows (idempotence, per-answer and whole-predicate
     cancellation, unknown-predicate silence), inactive-branch skipping,
     macro-produced assertions (resolved), never-expanded predicates/answers,
     assertions as ordinary integers in #if arithmetic, and the verbatim
     950919-1.c torture witness.
  2. The diagnostic battery: GCC 16.2's exact error spellings and exit codes
     for malformed assertions, plus the default-on -Wdeprecated warnings and
     their -Wno-deprecated silencing.  Column positions are deliberately not
     pinned (LCCC's caret style differs; both compilers reject).
  3. -A command-line semantics: -Apred=ans, the split "-A pred=ans" form,
     -A-pred=ans cancellation of exactly one predefined answer, and the
     measured oddity that a malformed spec prints a <command-line> error yet
     exits 0.
Usage:
  python3 tests/regression/check_gnu_assertions.py [--lccc PATH]
      [--gcc PATH] [--gcc32 PATH]
  python3 tests/regression/check_gnu_assertions.py --self-test
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# Corpus: (name, source, expected tokens on x86-64, expected tokens on i686).
# Expected values are the non-blank lines of `-E -P` output; None for an
# arch means the case is not run there.
# ---------------------------------------------------------------------------

TOKEN_CASES: list[tuple[str, str, list[str], list[str] | None]] = [
    (
        "predefined-cpu",
        "#if #cpu(x86_64)\nint c64;\n#else\nint not64;\n#endif\n",
        ["int c64;"],
        ["int not64;"],
    ),
    (
        "predefined-machine-and-i386",
        "#if #machine(x86_64)\nint m64;\n#endif\n"
        "#if #cpu(i386)\nint c386;\n#endif\n"
        "#if #machine(i386)\nint m386;\n#endif\n",
        ["int m64;"],
        ["int c386;", "int m386;"],
    ),
    (
        "predefined-system",
        "#if #system(linux)\nint l;\n#endif\n"
        "#if #system(unix)\nint u;\n#endif\n"
        "#if #system(posix)\nint p;\n#endif\n"
        "#if #system(windows)\nint w;\n#endif\n",
        ["int l;", "int u;", "int p;"],
        ["int l;", "int u;", "int p;"],
    ),
    (
        "bare-predicate",
        "#if #system\nint anysys;\n#endif\n#if #frobnicate\nint frob;\n#endif\n",
        ["int anysys;"],
        ["int anysys;"],
    ),
    (
        "whitespace-in-test",
        "# if # cpu ( x86_64 )\nint spaced;\n#endif\n"
        "#if #cpu\t(x86_64)\nint tabbed;\n#endif\n",
        ["int spaced;", "int tabbed;"],
        [],
    ),
    (
        "assert-basic-and-idempotent",
        "#assert food(tasty)\n#assert food(tasty)\n"
        "#if #food(tasty)\nint tasty;\n#endif\n"
        "#if #food(bland)\nint bland;\n#endif\n",
        ["int tasty;"],
        ["int tasty;"],
    ),
    (
        "answer-canonicalization",
        # Answer tokens are compared by spelling plus PRESENCE (not amount)
        # of inter-token whitespace (measured: `a   b` matches `a b`, but
        # `ab` does not, and `x , y` does not match `x, y`).
        "#assert pair(a   b)\n"
        "#if #pair(a b)\nint ab_space;\n#endif\n"
        "#if #pair(ab)\nint ab_fused;\n#endif\n"
        "#assert tup(x, y)\n"
        "#if #tup(x, y)\nint t_exact;\n#endif\n"
        "#if #tup(x , y)\nint t_spaced;\n#endif\n"
        "#if #tup(x,y)\nint t_tight;\n#endif\n",
        ["int ab_space;", "int t_exact;"],
        ["int ab_space;", "int t_exact;"],
    ),
    (
        "unassert-one-answer",
        "#assert tool(hammer)\n#assert tool(wrench)\n"
        "#unassert tool(hammer)\n"
        "#if #tool(hammer)\nint hammer;\n#endif\n"
        "#if #tool(wrench)\nint wrench;\n#endif\n"
        "#if #tool\nint anytool;\n#endif\n",
        ["int wrench;", "int anytool;"],
        ["int wrench;", "int anytool;"],
    ),
    (
        "unassert-whole-predicate-and-unknown",
        "#assert tool(hammer)\n#assert tool(wrench)\n"
        "#unassert tool\n"
        "#unassert neverwas\n"
        "#if #tool\nint anytool;\n#else\nint notool;\n#endif\n",
        ["int notool;"],
        ["int notool;"],
    ),
    (
        "unassert-predefined",
        "#unassert system(posix)\n"
        "#if #system(posix)\nint posix;\n#else\nint noposix;\n#endif\n"
        "#if #system(linux)\nint linux_;\n#endif\n",
        ["int noposix;", "int linux_;"],
        ["int noposix;", "int linux_;"],
    ),
    (
        "skipped-branch-ignores-directives",
        "#if 0\n#assert ghost(here)\n#endif\n"
        "#if #ghost(here)\nint ghost;\n#else\nint noghost;\n#endif\n",
        ["int noghost;"],
        ["int noghost;"],
    ),
    (
        "macro-produced-assertion",
        "#define CPUTEST #cpu(x86_64)\n"
        "#if CPUTEST\nint viamacro;\n#else\nint notmacro;\n#endif\n",
        ["int viamacro;"],
        ["int notmacro;"],
    ),
    (
        "predicate-and-answer-never-expanded",
        "#define cpu system\n#define x86_64 nonsense\n"
        "#if #cpu(x86_64)\nint still;\n#endif\n",
        ["int still;"],
        [],
    ),
    (
        "assertions-in-arithmetic",
        "#if #cpu(x86_64) + #system(linux) == 2\nint sum2;\n#endif\n"
        "#if !#cpu(m68k)\nint notm68k;\n#endif\n"
        "#if #cpu(x86_64) - 1\nint minus;\n#else\nint zero;\n#endif\n"
        "#if (#cpu(x86_64)) * 3 == 3\nint mul3;\n#else\nint mul0;\n#endif\n"
        "#if #cpu(m68k) ? 10 : 20\nint t20;\n#endif\n",
        ["int sum2;", "int notm68k;", "int zero;", "int mul3;", "int t20;"],
        # i686: #cpu(x86_64) is 0, so `0 - 1` is -1 (truthy) and `0*3 == 3`
        # is false -- the assertion really is an ordinary integer operand.
        ["int notm68k;", "int minus;", "int mul0;", "int t20;"],
    ),
    (
        "torture-950919-1",
        "#define empty\n#if empty#cpu(m68k)\n#endif\n\nvoid f (void){}\n",
        ["void f (void){}"],
        ["void f (void){}"],
    ),
    (
        "elif-and-nesting",
        "#if #cpu(m68k)\nint m68k;\n#elif #cpu(x86_64)\nint amd64;\n#elif #cpu(i386)\nint ia32;\n#else\nint other;\n#endif\n",
        ["int amd64;"],
        ["int ia32;"],
    ),
]

# (name, source, expected exit code, required stderr substrings,
#  forbidden stderr substrings)
DIAG_CASES: list[tuple[str, str, int, list[str], list[str]]] = [
    (
        "bare-hash",
        "#if #\n#endif\nint x;\n",
        1,
        ["assertion without predicate"],
        [],
    ),
    (
        "unterminated-answer",
        "#if #foo(unterminated\n#endif\nint x;\n",
        1,
        ["missing ')' to complete answer"],
        [],
    ),
    (
        "fused-operand",
        "#if 1#cpu(m68k)\n#endif\nint x;\n",
        1,
        ["missing binary operator before token '#'"],
        [],
    ),
    (
        "empty-answer",
        "#if #foo()\n#endif\nint x;\n",
        1,
        ["predicate's answer is empty"],
        [],
    ),
    (
        "assert-missing-paren",
        "#assert foo\nint x;\n",
        1,
        ["missing '(' after predicate", "'#assert' is a deprecated GCC extension"],
        [],
    ),
    (
        "bare-unassert",
        "#unassert\nint x;\n",
        1,
        ["assertion without predicate", "'#unassert' is a deprecated GCC extension"],
        [],
    ),
    (
        "assert-trailing-junk",
        "#assert foo(x) junk\n#if #foo(x)\nint ok;\n#endif\n",
        0,
        ["extra tokens at end of '#assert' directive"],
        [],
    ),
    (
        "deprecation-warning-default-on",
        "#if #cpu(x86_64)\nint a;\n#endif\n",
        0,
        ["assertions are a deprecated extension", "-Wdeprecated"],
        [],
    ),
]

# (name, extra argv, source, expected exit, expected tokens,
#  required stderr substrings, forbidden stderr substrings)
CLI_CASES: list[tuple[str, list[str], str, int, list[str], list[str], list[str]]] = [
    (
        "cli-assert-joined",
        ["-Amypred=myans"],
        "#if #mypred(myans)\nint got;\n#endif\n",
        0,
        ["int got;"],
        [],
        [],
    ),
    (
        "cli-assert-split",
        ["-A", "mypred=myans"],
        "#if #mypred(myans)\nint got;\n#endif\n",
        0,
        ["int got;"],
        [],
        [],
    ),
    (
        "cli-cancel-one-predefined",
        ["-A-system=linux"],
        "#if #system(linux)\nint linux_;\n#else\nint nolinux;\n#endif\n"
        "#if #system(unix)\nint unix_;\n#endif\n",
        0,
        ["int nolinux;", "int unix_;"],
        [],
        [],
    ),
    (
        "cli-assert-new-system-answer",
        ["-A", "system=bsd"],
        "#if #system(bsd)\nint bsd;\n#endif\n#if #system(linux)\nint linux_;\n#endif\n",
        0,
        ["int bsd;", "int linux_;"],
        [],
        [],
    ),
    (
        "cli-malformed-spec-exits-zero",
        ["-Afoo"],
        "int plain;\n",
        0,
        ["int plain;"],
        ["<command-line>: error: missing '(' after predicate"],
        [],
    ),
    (
        "wno-deprecated-silences",
        ["-Wno-deprecated"],
        "#if #cpu(x86_64)\nint a;\n#else\nint b;\n#endif\n"
        "#assert q(r)\n#unassert q\n",
        0,
        None,  # arch-dependent tokens; stderr is the subject here
        [],
        ["deprecated"],
    ),
]


def run_compiler(
    compiler: str, args: list[str], source: str, tmpdir: str, name: str
) -> tuple[int, list[str], str]:
    """Preprocess `source`; return (exit, non-blank stdout lines, stderr)."""
    src = Path(tmpdir) / f"{name}.c"
    src.write_text(source)
    proc = subprocess.run(
        [compiler, "-E", "-P", *args, str(src)],
        capture_output=True,
        text=True,
        timeout=60,
    )
    tokens = [line for line in proc.stdout.splitlines() if line.strip()]
    return proc.returncode, tokens, proc.stderr


class Gate:
    def __init__(self) -> None:
        self.failures: list[str] = []
        self.checks = 0

    def expect(self, cond: bool, label: str, detail: str = "") -> None:
        self.checks += 1
        if not cond:
            self.failures.append(f"{label}: {detail}" if detail else label)

    def report(self, title: str) -> bool:
        if self.failures:
            print(f"FAIL {title}: {len(self.failures)} of {self.checks} checks")
            for failure in self.failures:
                print(f"  FAIL {failure}")
            return False
        print(f"  PASS {title} ({self.checks} checks)")
        return True


def check_token_corpus(
    gate: Gate, compiler: str, arch_args: list[str], column: int, tmpdir: str, tag: str
) -> None:
    for name, source, exp64, exp32 in TOKEN_CASES:
        expected = exp64 if column == 0 else exp32
        if expected is None:
            continue
        code, tokens, _ = run_compiler(compiler, arch_args, source, tmpdir, name)
        gate.expect(code == 0, f"{tag}:{name}", f"exit {code}, wanted 0")
        gate.expect(
            tokens == expected, f"{tag}:{name}", f"tokens {tokens} != {expected}"
        )


def check_differential(
    gate: Gate,
    lccc: str,
    oracle: str,
    lccc_args: list[str],
    oracle_args: list[str],
    tmpdir: str,
    tag: str,
) -> None:
    """The oracle must agree with the PIN, not merely with LCCC."""
    for name, source, exp64, exp32 in TOKEN_CASES:
        expected = exp64 if "-m32" not in oracle_args else exp32
        if expected is None:
            continue
        ocode, otokens, _ = run_compiler(oracle, oracle_args, source, tmpdir, name)
        gate.expect(
            ocode == 0 and otokens == expected,
            f"{tag}:{name}",
            f"oracle produced exit {ocode} tokens {otokens}, pin says {expected}"
            " -- the PIN is wrong or the oracle moved",
        )
        lcode, ltokens, _ = run_compiler(lccc, lccc_args, source, tmpdir, name)
        gate.expect(
            (lcode, ltokens) == (ocode, otokens),
            f"{tag}:{name}",
            f"lccc (exit {lcode}, {ltokens}) != oracle (exit {ocode}, {otokens})",
        )


def check_diagnostics(gate: Gate, compiler: str, tmpdir: str, tag: str) -> None:
    for name, source, exp_code, needed, forbidden in DIAG_CASES:
        code, _, stderr = run_compiler(compiler, [], source, tmpdir, name)
        gate.expect(
            code == exp_code, f"{tag}:{name}", f"exit {code}, wanted {exp_code}"
        )
        for text in needed:
            gate.expect(
                text in stderr,
                f"{tag}:{name}",
                f"stderr missing {text!r}; got: {stderr.strip()!r}",
            )
        for text in forbidden:
            gate.expect(
                text not in stderr,
                f"{tag}:{name}",
                f"stderr unexpectedly contains {text!r}",
            )


def check_cli(gate: Gate, compiler: str, tmpdir: str, tag: str) -> None:
    for name, argv, source, exp_code, exp_tokens, needed, forbidden in CLI_CASES:
        code, tokens, stderr = run_compiler(compiler, argv, source, tmpdir, name)
        gate.expect(
            code == exp_code, f"{tag}:{name}", f"exit {code}, wanted {exp_code}"
        )
        if exp_tokens is not None:
            gate.expect(
                tokens == exp_tokens,
                f"{tag}:{name}",
                f"tokens {tokens} != {exp_tokens}",
            )
        for text in needed:
            gate.expect(
                text in stderr,
                f"{tag}:{name}",
                f"stderr missing {text!r}; got: {stderr.strip()!r}",
            )
        for text in forbidden:
            gate.expect(
                text not in stderr,
                f"{tag}:{name}",
                f"stderr unexpectedly contains {text!r}",
            )


def self_test(lccc: str) -> int:
    """Prove every check class can fail: feed each comparator a sabotaged
    expectation against the real compiler and require a recorded failure."""
    with tempfile.TemporaryDirectory() as tmpdir:
        # 1. A wrong token pin must fail.
        gate = Gate()
        code, tokens, _ = run_compiler(
            lccc, [], "#if #cpu(x86_64)\nint a;\n#else\nint b;\n#endif\n", tmpdir, "st1"
        )
        gate.expect(code == 0, "st1-exit")
        gate.expect(tokens == ["int DELIBERATELY_WRONG;"], "st1-tokens")
        if not gate.failures:
            print("self-test FAILED: wrong token pin not detected")
            return 1
        # 2. A wrong exit-code pin must fail.
        gate = Gate()
        code, _, _ = run_compiler(lccc, [], "#if #\n#endif\n", tmpdir, "st2")
        gate.expect(code == 0, "st2-exit")  # real exit is 1
        if not gate.failures:
            print("self-test FAILED: wrong exit pin not detected")
            return 1
        # 3. A wrong stderr requirement must fail.
        gate = Gate()
        _, _, stderr = run_compiler(lccc, [], "int x;\n", tmpdir, "st3")
        gate.expect("no such diagnostic" in stderr, "st3-stderr")
        if not gate.failures:
            print("self-test FAILED: wrong stderr pin not detected")
            return 1
        # 4. The forbidden-text direction must fail when the text IS present.
        gate = Gate()
        _, _, stderr = run_compiler(
            lccc, [], "#if #cpu(x86_64)\n#endif\nint x;\n", tmpdir, "st4"
        )
        gate.expect("deprecated" not in stderr, "st4-forbidden")
        if not gate.failures:
            print("self-test FAILED: forbidden-text pin not detected")
            return 1
    print("self-test: all four sabotaged expectations were detected: PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--lccc", default=os.environ.get("CCC", "target/fastbuild/lccc")
    )
    parser.add_argument(
        "--gcc", help="GCC oracle for the differential x86-64 lane (e.g. gcc-16)"
    )
    parser.add_argument(
        "--gcc32", help="GCC oracle for the differential -m32 lane (e.g. gcc-16)"
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test(args.lccc)

    ok = True
    with tempfile.TemporaryDirectory() as tmpdir:
        gate = Gate()
        check_token_corpus(gate, args.lccc, [], 0, tmpdir, "x86_64")
        ok &= gate.report("pinned token corpus, x86-64")

        gate = Gate()
        check_token_corpus(gate, args.lccc, ["-m32"], 1, tmpdir, "i686")
        ok &= gate.report("pinned token corpus, i686 (-m32)")

        gate = Gate()
        check_diagnostics(gate, args.lccc, tmpdir, "diag")
        ok &= gate.report("diagnostic battery (GCC 16.2 spellings and exits)")

        gate = Gate()
        check_cli(gate, args.lccc, tmpdir, "cli")
        ok &= gate.report("-A command-line semantics")

        if args.gcc:
            gate = Gate()
            check_differential(gate, args.lccc, args.gcc, [], [], tmpdir, "diff64")
            ok &= gate.report(f"differential vs {args.gcc}, x86-64")
        if args.gcc32:
            gate = Gate()
            check_differential(
                gate, args.lccc, args.gcc32, ["-m32"], ["-m32"], tmpdir, "diff32"
            )
            ok &= gate.report(f"differential vs {args.gcc32} -m32, i686")

    if not ok:
        return 1
    print("GNU_ASSERTIONS_OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
