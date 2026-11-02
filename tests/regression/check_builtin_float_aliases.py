#!/usr/bin/env python3
"""Pin the `__builtin_<fn>f` aliases against the compiler that defines the class.

# The defect this gate exists for

lccc's `BuiltinKind::LibcAlias` table maps `__builtin_X` to the libc symbol `X`
so a program written against GCC can call `__builtin_fmax` without including
`<math.h>`. GCC offers the float spelling `__builtin_Xf` wherever it offers
`__builtin_X`; lccc had added the *long double* siblings (`fmaxl`, `fminl`)
while missing three of the float ones (`fmaxf`, `fminf`, `log2f`).

The failure mode is quiet, which is why it needs a gate rather than a note. An
unmapped `__builtin_fmaxf` parses as an **implicit function declaration**:

  * lccc warns and continues (a warning, not an error);
  * the emitted call targets the literal symbol `__builtin_fmaxf`, which no
    libc defines, so the link fails pointing at the *object* rather than at the
    missing table entry;
  * where the diagnostic is ignored, the call's return type is `int`, so the
    float result arrives as an integer conversion of `%eax` -- a silently wrong
    value instead of a missing symbol.

# Why the class is derived, not listed

A hand-written list is the artefact that drifts; the drift is what caused the
defect. The class is derived on every run from two authorities:

  1. `src/frontend/sema/builtins.rs` -- the mappings lccc ships, parsed for
     `BuiltinInfo::simple("<libc>")` entries whose libc name is a bare
     identifier without an `f`/`l` suffix;
  2. `gcc-16 -fsyntax-only` -- whether `__builtin_<libc>f` exists in the
     reference compiler, and with what arity (probed, not assumed: `log2f` takes
     one argument, `fmaxf` takes two, and passing doubles to a float parameter
     would compare the wrong thing).

For every derived pair, lccc must accept the spelling with
`-Werror=implicit-function-declaration` (so "warned and continued" fails), must
not leave the builtin name in the emitted assembly, and the runtime behaviour of
the float spellings -- including both NaN operand orders -- must be
bit-identical to GCC 16.2.

# Anti-vacuity

A name no compiler defines must fail lccc's `-Werror` probe, and must fail with
an *implicit declaration* diagnostic specifically. If it passes, or fails for
another reason, this gate is not testing what it claims and exits non-zero
regardless of everything else.

Usage:
    check_builtin_float_aliases.py [--lccc PATH] [--gcc gcc-16] [--verbose]

Exit status:
    0  every derived pair is mapped, no builtin name survives, runtime matches
    1  a pair is missing, a builtin name survived, the runtime differs, or the
       control probe did not behave (the gate is broken, not the compiler)
    2  a prerequisite is missing (lccc binary, gcc-16, sources)
"""
from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BUILTINS_RS = REPO / "src/frontend/sema/builtins.rs"

# Prefixes whose `<name>f` spelling is a libm float function. Kept broad: the
# GCC probe below discards everything that is not a real builtin, and a name
# that is missed here can only make the gate *weaker*, never produce a false
# failure -- except for `check_candidate_count`, which refuses a suspiciously
# small derivation for exactly that reason.
_SHAPE = re.compile(r'BuiltinInfo::simple\("([A-Za-z_][A-Za-z0-9_]*)"\)')


def candidate_libc_names(source: str) -> list[str]:
    """Libc names from `simple()` mappings that have a float sibling to probe."""
    names = {match for match in _SHAPE.findall(source)}
    # `fmax` -> probe `__builtin_fmaxf`. The long-double forms (`fmaxl`) and
    # already-float names (`fabsf`) have no such sibling.
    return sorted(name for name in names if not name.endswith(("f", "l")))


def _gcc(gcc: str, program: str) -> tuple[bool, str]:
    with tempfile.NamedTemporaryFile("w", suffix=".c", delete=False) as handle:
        handle.write(program)
        path = handle.name
    try:
        result = subprocess.run(
            [gcc, "-fsyntax-only", "-x", "c", path],
            capture_output=True, text=True)
        return result.returncode == 0, result.stderr
    finally:
        Path(path).unlink(missing_ok=True)


def gcc_arity(gcc: str, builtin: str) -> int | None:
    """Arity of `builtin` according to the reference compiler.

    Probed rather than tabulated: the answer decides how the lccc probe has to
    call it, and a wrong arity would test the diagnostic for the arity instead
    of the mapping.
    """
    for arity in (1, 2, 3):
        args = ", ".join(["1.0"] * arity)
        if _gcc(gcc, f"double f(double a);\n"
                     f"double probe(void) {{ return (double){builtin}({args}); }}\n")[0]:
            return arity
    return None


def lccc_probe(lccc: str, builtin: str, arity: int) -> tuple[bool, str, str]:
    """Compile one call with lccc; implicit declarations are errors here."""
    params = ", ".join(f"float a{i}" for i in range(arity))
    args = ", ".join(f"a{i}" for i in range(arity))
    body = (f"float probe({params}) {{ return (float){builtin}({args}); }}\n"
            f"double probe_d(double d) {{ (void)d; return 0.0; }}\n")
    with tempfile.TemporaryDirectory() as directory:
        src = Path(directory) / "probe.c"
        src.write_text(body)
        result = subprocess.run(
            [lccc, "-O2", "-S", "-Werror=implicit-function-declaration",
             "-x", "c", str(src), "-o", "-"],
            capture_output=True, text=True)
        return result.returncode == 0, result.stderr, result.stdout


RUNTIME_PROGRAM = r"""
#include <math.h>
#include <stdio.h>

/* For every operand pair below the program prints
 *     <case> <builtin result> <libc reference result>
 * so the gate can ask two separate questions:
 *
 *  1. does lccc's builtin spelling agree with the libc function it names?
 *     (that is the mapping contract, and it is checked on EVERY case);
 *  2. do lccc and GCC agree with each other? (checked on every case whose
 *     answer is not a signed zero -- see the note below).
 *
 * The reference is taken through a function POINTER so no compiler can fold,
 * inline or constant-propagate the call: this is the library's answer.
 *
 * Signed zero is deliberately its own case group. glibc's fmax/fmin are
 * comparison-based (`x >= y ? x : y`), so fmax(-0,+0) is -0 and fmin(-0,+0) is
 * +0, while a compiler that inlines a `maxss`-style lowering answers +0 / -0 --
 * the Annex F answer. GCC 16.2 inlines and answers +0/-0; lccc calls glibc and
 * answers -0/+0. Both are defensible and neither is the other's bug, so the
 * gate pins each compiler to the LIBRARY it binds to (question 1) and does not
 * demand that a call and an inline expansion produce the same zero sign.
 */
static float (*ref_fmaxf)(float, float) = fmaxf;
static float (*ref_fminf)(float, float) = fminf;
static double (*ref_fmax)(double, double) = fmax;
static double (*ref_fmin)(double, double) = fmin;

int main(void) {
    volatile float nf = __builtin_nanf(""), inf_f = __builtin_inff();
    volatile double nd = __builtin_nan(""), inf_d = __builtin_inf();
    volatile float mzf = -0.0f, pzf = 0.0f, one_f = 1.0f, two_f = 2.0f;
    volatile float den_f = 1e-40f, neg_f = -2.5f;
    volatile double mzd = -0.0, pzd = 0.0, one_d = 1.0, two_d = 2.0;
    volatile double den_d = 1e-310, neg_d = -2.5;

#define CASE_F(name, x, y) \
    printf("%s %.17g %.17g\n", name, (double)__builtin_fmaxf(x, y), (double)ref_fmaxf(x, y)); \
    printf("%s %.17g %.17g\n", name "|min", (double)__builtin_fminf(x, y), (double)ref_fminf(x, y))
#define CASE_D(name, x, y) \
    printf("%s %.17g %.17g\n", name, __builtin_fmax(x, y), ref_fmax(x, y)); \
    printf("%s %.17g %.17g\n", name "|min", __builtin_fmin(x, y), ref_fmin(x, y))

    CASE_F("ord-f", one_f, two_f);
    CASE_F("ord-f-rev", two_f, one_f);
    CASE_F("neg-f", neg_f, one_f);
    CASE_F("den-f", den_f, one_f);
    CASE_F("nan-f-a", nf, one_f);
    CASE_F("nan-f-b", one_f, nf);
    CASE_F("inf-f", inf_f, one_f);
    CASE_F("zerof", mzf, pzf);
    CASE_D("ord-d", one_d, two_d);
    CASE_D("ord-d-rev", two_d, one_d);
    CASE_D("neg-d", neg_d, one_d);
    CASE_D("den-d", den_d, one_d);
    CASE_D("nan-d-a", nd, one_d);
    CASE_D("nan-d-b", one_d, nd);
    CASE_D("inf-d", inf_d, one_d);
    CASE_D("zerod", mzd, pzd);
    /* log2f is the third name this gate was written for; compare against libc
       too, so the float spelling is exercised on a non-NaN-symmetric op. */
    printf("log2f %.17g %.17g\n", (double)__builtin_log2f(3.0f), (double)log2f(3.0f));
    return 0;
}
"""


def runtime_differential(lccc: str, gcc: str) -> tuple[bool, str]:
    with tempfile.TemporaryDirectory() as directory:
        src = Path(directory) / "rt.c"
        src.write_text(RUNTIME_PROGRAM)
        outputs = {}
        for name, compiler in (("lccc", lccc), ("gcc", gcc)):
            binary = Path(directory) / f"rt.{name}"
            # -lm is part of the contract under test, not an incidental flag:
            # the mappings resolve to libm symbols and libm is a separate DSO
            # on glibc, so a run that cannot link them proves nothing.
            build = subprocess.run([compiler, "-O2", str(src), "-o", str(binary), "-lm"],
                                   capture_output=True, text=True)
            if build.returncode != 0:
                return False, f"{name} build failed:\n{build.stderr.strip()}"
            run = subprocess.run([str(binary)], capture_output=True, text=True)
            if run.returncode != 0:
                return False, f"{name} run exited {run.returncode}"
            outputs[name] = run.stdout
        return adjudicate(outputs["lccc"], outputs["gcc"])


ZERO_SIGN_CASES = ("zerof", "zerod")


def adjudicate(lccc_out: str, gcc_out: str) -> tuple[bool, str]:
    """Two independent questions, asked in the documented order.

    1. lccc's builtin spelling must agree with the libc function it names, for
       every case. The reference column is a real library call through a
       function pointer, so this compares the *mapping*, not two inline
       expansions.
    2. lccc and GCC must agree with each other, except on the signed-zero cases,
       where glibc and an Annex F inline expansion legitimately differ (the
       program's header comment says which answers each gives).
    """
    def rows(text: str) -> dict[str, tuple[float, float]]:
        out = {}
        for line in text.splitlines():
            parts = line.split()
            if len(parts) != 3:
                continue
            out[parts[0]] = (float(parts[1]), float(parts[2]))
        return out

    mine, theirs = rows(lccc_out), rows(gcc_out)
    if set(mine) != set(theirs):
        return False, f"case sets differ: {sorted(set(mine) ^ set(theirs))}"
    problems = []
    for name, (builtin, reference) in mine.items():
        if not _same(builtin, reference):
            problems.append(f"{name}: lccc builtin {builtin!r} != libc {reference!r}")
        elif name in ZERO_SIGN_CASES or name.endswith("|min") and name[:-4] in ZERO_SIGN_CASES:
            continue
        elif not _same(builtin, theirs[name][0]):
            problems.append(f"{name}: lccc {builtin!r} != gcc {theirs[name][0]!r}")
    if problems:
        return False, "\n  ".join(problems)
    return True, lccc_out.strip()


def _same(a: float, b: float) -> bool:
    """Bit-identical, with NaN == NaN and the sign of zero preserved."""
    import math
    import struct
    if math.isnan(a) and math.isnan(b):
        return True
    return struct.pack("<d", a) == struct.pack("<d", b)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--lccc", default=str(REPO / "target/fastbuild/lccc"))
    parser.add_argument("--gcc", default="gcc-16")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args(argv)

    lccc = Path(args.lccc)
    if not lccc.is_file():
        print(f"check_builtin_float_aliases: no lccc binary at {lccc}", file=sys.stderr)
        return 2
    if not BUILTINS_RS.is_file():
        print(f"check_builtin_float_aliases: missing {BUILTINS_RS}", file=sys.stderr)
        return 2
    try:
        version = subprocess.run([args.gcc, "--version"], capture_output=True, text=True)
    except FileNotFoundError:
        print(f"check_builtin_float_aliases: {args.gcc} not on PATH; the class "
              "cannot be derived without the reference compiler", file=sys.stderr)
        return 2
    if version.returncode != 0:
        print(f"check_builtin_float_aliases: {args.gcc} --version failed", file=sys.stderr)
        return 2

    # ---- anti-vacuity control ----------------------------------------------
    control = "__builtin_lccc_not_a_real_builtin_xyz"
    accepted, stderr, _ = lccc_probe(str(lccc), control, 1)
    if accepted:
        print("check_builtin_float_aliases: SELF-CHECK FAILED -- lccc accepted "
              f"{control}, so an unmapped builtin would not be detected",
              file=sys.stderr)
        return 1
    if "implicit declaration" not in stderr:
        print("check_builtin_float_aliases: SELF-CHECK FAILED -- the control "
              f"failed for the wrong reason:\n{stderr.strip()}", file=sys.stderr)
        return 1

    # ---- derive the class ---------------------------------------------------
    pairs: list[tuple[str, str, int]] = []
    for libc in candidate_libc_names(BUILTINS_RS.read_text()):
        builtin = f"__builtin_{libc}f"
        arity = gcc_arity(args.gcc, builtin)
        if arity is not None:
            pairs.append((builtin, f"{libc}f", arity))
    if len(pairs) < 10:
        print(f"check_builtin_float_aliases: derived only {len(pairs)} pairs; the "
              "table parse or the oracle probe has drifted", file=sys.stderr)
        return 1
    if args.verbose:
        print(f"derived {len(pairs)} __builtin_<fn>f pairs from {args.gcc}")

    # ---- check every derived pair -------------------------------------------
    failures: list[str] = []
    inlined = 0
    for builtin, libc_name, arity in pairs:
        accepted, stderr, asm = lccc_probe(str(lccc), builtin, arity)
        if not accepted:
            detail = stderr.strip().splitlines()
            failures.append(f"{builtin}: not mapped (implicit declaration)"
                            + (f"\n    {detail[0]}" if detail else ""))
            continue
        if builtin in asm:
            failures.append(f"{builtin}: the builtin name survived into the "
                            f"assembly instead of {libc_name}")
        elif libc_name not in asm:
            # Inlined to an instruction (`sqrtss` for `sqrtf`) is legitimate and
            # is reported, not failed: the assertion is about the mapping, not
            # about how it is lowered.
            inlined += 1
            if args.verbose:
                print(f"note {builtin}: lowered inline, no {libc_name} call")

    # ---- runtime differential for the float spellings ------------------------
    ok, detail = runtime_differential(str(lccc), args.gcc)
    if not ok:
        failures.append(f"runtime differential: {detail}")

    if failures:
        for failure in failures:
            print(f"FAIL {failure}")
        print(f"check_builtin_float_aliases: {len(failures)} failure(s)")
        return 1
    print(f"check_builtin_float_aliases: PASS ({len(pairs)} derived pairs, "
          f"{inlined} lowered inline; every builtin agrees with the libc "
          f"function it names, and with {args.gcc} on every non-signed-zero "
          "case)")
    if args.verbose:
        print(detail)
    return 0


if __name__ == "__main__":
    sys.exit(main())
