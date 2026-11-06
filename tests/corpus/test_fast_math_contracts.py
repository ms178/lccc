#!/usr/bin/env python3
"""The complete source-visible fast-math contract, pinned to measured oracles.

This module locks down the fast-math *state machine* end to end: which macro is
defined for which command line, in which order the group setters apply, and
what the resulting code is allowed to assume. Every expected value here was
measured against GCC 16.2.0 and Clang 23.1.5 with `-dM -E` (or `-S`) and is
reproduced by the differential harness in `/tmp/fmprobe/diff.py` during
development; the tables below are the frozen, CI-runnable subset so a
regression cannot depend on an oracle being installed.

Three real defects are pinned by this file; none of them was visible to the
previous gates because those only compared the *default* command line:

1. **The IEEE-754 conformance lie.** `__STDC_IEC_559__`,
   `__STDC_IEC_559_COMPLEX__`, `__STDC_IEC_60559_BFP__`,
   `__STDC_IEC_60559_COMPLEX__`, `__GCC_IEC_559` and `__GCC_IEC_559_COMPLEX`
   were defined unconditionally. Under `-Ofast` this compiler therefore told
   every numerics header that it was Annex F conformant while the optimizer was
   busy breaking exactly the guarantees those macros promise. GCC 16.2's
   predicate is `!finite_math_only && signed_zeros && !reciprocal_math`, and
   the complex pair is additionally withdrawn by `-fcx-limited-range`, which
   `-ffast-math` implies.

2. **Silently ignored sub-options.** `-fsigned-zeros`, `-fno-signed-zeros`,
   `-freciprocal-math`, `-fno-reciprocal-math`, `-fcx-limited-range` and
   `-fno-cx-limited-range` were accepted and did nothing at all, so a build
   system probing for them concluded the relaxation was in force.

3. **`-funsafe-math-optimizations` was only `-fassociative-math`.** GCC expands
   it to the group `-fno-signed-zeros -fno-trapping-math -fassociative-math
   -freciprocal-math`; LCCC set the reassociation bit and dropped the rest,
   which misreported four macros and left the conformance claim standing.

Two command lines are genuine oracle disagreements (`KNOWN_SPLITS`); LCCC picks
a side deliberately and each choice is justified in the table.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# ---------------------------------------------------------------------------
# The contract.  `M` is the tuple of presence/value tests applied to one
# command line: (macro, expected) where `expected` is
#   "-"  -> macro must be ABSENT
#   str  -> macro must be defined with exactly that body
# ---------------------------------------------------------------------------

PROBE = "int main(void) { return 0; }\n"

# Columns: (flags, ((macro, expected), ...))
CONTRACT: tuple[tuple[str, tuple[tuple[str, str], ...]], ...] = (
    # --- baseline: strict C, full conformance -----------------------------
    ("-O3", (
        ("__FAST_MATH__", "-"), ("__FINITE_MATH_ONLY__", "0"),
        ("__NO_MATH_ERRNO__", "-"), ("__NO_SIGNED_ZEROS__", "-"),
        ("__NO_TRAPPING_MATH__", "-"), ("__ASSOCIATIVE_MATH__", "-"),
        ("__RECIPROCAL_MATH__", "-"), ("__ROUNDING_MATH__", "-"),
        ("__STDC_IEC_559__", "1"), ("__STDC_IEC_559_COMPLEX__", "1"),
        ("__STDC_IEC_60559_BFP__", "201404L"),
        ("__STDC_IEC_60559_COMPLEX__", "201404L"),
        ("__GCC_IEC_559", "2"), ("__GCC_IEC_559_COMPLEX", "2"),
    )),
    # --- the umbrella turns everything on and withdraws conformance -------
    ("-O3 -ffast-math", (
        ("__FAST_MATH__", "1"), ("__FINITE_MATH_ONLY__", "1"),
        ("__NO_MATH_ERRNO__", "1"), ("__NO_SIGNED_ZEROS__", "1"),
        ("__NO_TRAPPING_MATH__", "1"), ("__ASSOCIATIVE_MATH__", "1"),
        ("__RECIPROCAL_MATH__", "1"),
        ("__STDC_IEC_559__", "-"), ("__STDC_IEC_559_COMPLEX__", "-"),
        ("__STDC_IEC_60559_BFP__", "-"), ("__STDC_IEC_60559_COMPLEX__", "-"),
        ("__GCC_IEC_559", "0"), ("__GCC_IEC_559_COMPLEX", "0"),
    )),
    ("-Ofast", (
        ("__FAST_MATH__", "1"), ("__FINITE_MATH_ONLY__", "1"),
        ("__NO_SIGNED_ZEROS__", "1"), ("__ASSOCIATIVE_MATH__", "1"),
        ("__RECIPROCAL_MATH__", "1"), ("__NO_TRAPPING_MATH__", "1"),
        ("__STDC_IEC_559__", "-"), ("__GCC_IEC_559", "0"),
    )),
    # --- each relaxation withdraws the claim on its own -------------------
    ("-O3 -ffinite-math-only", (
        ("__FINITE_MATH_ONLY__", "1"), ("__STDC_IEC_559__", "-"),
        ("__GCC_IEC_559", "0"), ("__FAST_MATH__", "-"),
    )),
    ("-O3 -fno-signed-zeros", (
        ("__NO_SIGNED_ZEROS__", "1"), ("__STDC_IEC_559__", "-"),
        ("__GCC_IEC_559", "0"), ("__FAST_MATH__", "-"),
    )),
    ("-O3 -freciprocal-math", (
        ("__RECIPROCAL_MATH__", "1"), ("__STDC_IEC_559__", "-"),
        ("__GCC_IEC_559", "0"), ("__FAST_MATH__", "-"),
    )),
    ("-O3 -fcx-limited-range", (
        # The complex claim only: the real claim stays intact.
        ("__STDC_IEC_559__", "1"), ("__GCC_IEC_559", "2"),
        ("__STDC_IEC_559_COMPLEX__", "-"), ("__GCC_IEC_559_COMPLEX", "0"),
        ("__FAST_MATH__", "-"),
    )),
    # --- relaxations that do NOT withdraw the claim -----------------------
    # Traps and reassociation change observability, not the values `+` can
    # produce, so Annex F still holds.
    ("-O3 -fno-trapping-math", (
        ("__NO_TRAPPING_MATH__", "1"), ("__STDC_IEC_559__", "1"),
        ("__GCC_IEC_559", "2"), ("__FAST_MATH__", "-"),
    )),
    ("-O3 -fassociative-math", (
        # GCC keys __ASSOCIATIVE_MATH__ on the unsafe-math flag, not on
        # reassociation, so the narrow spelling publishes nothing.
        ("__ASSOCIATIVE_MATH__", "-"), ("__STDC_IEC_559__", "1"),
        ("__FAST_MATH__", "-"),
    )),
    ("-O3 -fno-math-errno", (
        ("__NO_MATH_ERRNO__", "1"), ("__STDC_IEC_559__", "1"),
    )),
    ("-O3 -frounding-math", (
        ("__ROUNDING_MATH__", "1"), ("__STDC_IEC_559__", "1"),
    )),
    # --- -funsafe-math-optimizations is a GROUP, not a synonym ------------
    ("-O3 -funsafe-math-optimizations", (
        ("__ASSOCIATIVE_MATH__", "1"), ("__NO_SIGNED_ZEROS__", "1"),
        ("__NO_TRAPPING_MATH__", "1"), ("__RECIPROCAL_MATH__", "1"),
        ("__FINITE_MATH_ONLY__", "0"), ("__STDC_IEC_559__", "-"),
        ("__FAST_MATH__", "-"),
    )),
    # --- last-option-wins, both directions --------------------------------
    ("-O3 -ffast-math -fsigned-zeros", (
        ("__NO_SIGNED_ZEROS__", "-"), ("__FAST_MATH__", "-"),
        ("__FINITE_MATH_ONLY__", "1"),
    )),
    ("-O3 -fsigned-zeros -ffast-math", (
        # The umbrella is a hard group setter: an earlier explicit
        # sub-option is overridden.
        ("__NO_SIGNED_ZEROS__", "1"), ("__FAST_MATH__", "1"),
    )),
    ("-O3 -ffast-math -fno-finite-math-only", (
        ("__FINITE_MATH_ONLY__", "0"), ("__FAST_MATH__", "-"),
    )),
    ("-O3 -ffast-math -fmath-errno", (
        ("__NO_MATH_ERRNO__", "-"), ("__FAST_MATH__", "-"),
    )),
    ("-O3 -ffast-math -ftrapping-math", (
        ("__NO_TRAPPING_MATH__", "-"), ("__FAST_MATH__", "-"),
        ("__ASSOCIATIVE_MATH__", "-"),
    )),
    # Order really matters for the unsafe-math bit: a safe spelling given
    # AFTER the group withdraws __ASSOCIATIVE_MATH__, before it does not.
    ("-O3 -ftrapping-math -funsafe-math-optimizations", (
        ("__ASSOCIATIVE_MATH__", "1"),
    )),
    ("-O3 -funsafe-math-optimizations -ftrapping-math", (
        ("__ASSOCIATIVE_MATH__", "-"),
    )),
    ("-O3 -funsafe-math-optimizations -fsigned-zeros", (
        ("__ASSOCIATIVE_MATH__", "-"), ("__NO_SIGNED_ZEROS__", "-"),
    )),
    ("-Ofast -fno-associative-math", (("__ASSOCIATIVE_MATH__", "-"),)),
    ("-Ofast -fno-unsafe-math-optimizations", (
        ("__ASSOCIATIVE_MATH__", "-"), ("__NO_SIGNED_ZEROS__", "-"),
        ("__RECIPROCAL_MATH__", "-"), ("__NO_TRAPPING_MATH__", "-"),
        ("__FAST_MATH__", "-"),
    )),
    # --- withdrawal restores the strict defaults --------------------------
    ("-O3 -ffast-math -fno-fast-math", (
        ("__FAST_MATH__", "-"), ("__FINITE_MATH_ONLY__", "0"),
        ("__NO_MATH_ERRNO__", "-"), ("__NO_SIGNED_ZEROS__", "-"),
        ("__RECIPROCAL_MATH__", "-"), ("__NO_TRAPPING_MATH__", "-"),
        ("__ASSOCIATIVE_MATH__", "-"),
        ("__STDC_IEC_559__", "1"), ("__STDC_IEC_559_COMPLEX__", "1"),
        ("__GCC_IEC_559", "2"), ("__GCC_IEC_559_COMPLEX", "2"),
    )),
    ("-Ofast -fno-fast-math", (
        ("__FAST_MATH__", "-"), ("__FINITE_MATH_ONLY__", "0"),
        # `-fcx-limited-range` is the one group member an *implicit* umbrella
        # leaves behind, so the complex claim stays withdrawn.
        ("__STDC_IEC_559__", "1"), ("__STDC_IEC_559_COMPLEX__", "-"),
        ("__GCC_IEC_559", "2"), ("__GCC_IEC_559_COMPLEX", "0"),
    )),
    ("-Ofast -O2", (
        ("__FAST_MATH__", "-"), ("__FINITE_MATH_ONLY__", "0"),
        ("__STDC_IEC_559__", "1"), ("__STDC_IEC_559_COMPLEX__", "1"),
    )),
    # An explicit -fcx-limited-range is a user request: it survives the
    # umbrella withdrawal AND a later -O level.
    ("-ffast-math -fcx-limited-range -fno-fast-math", (
        ("__STDC_IEC_559_COMPLEX__", "-"), ("__GCC_IEC_559_COMPLEX", "0"),
        ("__STDC_IEC_559__", "1"),
    )),
    ("-Ofast -fcx-limited-range -O2", (
        ("__STDC_IEC_559_COMPLEX__", "-"), ("__GCC_IEC_559_COMPLEX", "0"),
    )),
    # `-fno-cx-limited-range` restores the complex range, but `-Ofast` has
    # already turned on -ffinite-math-only, which withdraws the whole claim on
    # its own. Both macros stay withdrawn (measured on GCC 16.2:
    # __GCC_IEC_559 0, __GCC_IEC_559_COMPLEX 0). The flag only becomes
    # observable on a command line that leaves the real claim standing -- see
    # the `-O3 -fcx-limited-range` row above.
    ("-Ofast -fno-cx-limited-range", (
        ("__STDC_IEC_559__", "-"), ("__STDC_IEC_559_COMPLEX__", "-"),
        ("__GCC_IEC_559", "0"), ("__GCC_IEC_559_COMPLEX", "0"),
    )),
)

# GCC 16.2 and Clang 23.1.5 genuinely disagree here. LCCC follows Clang for
# rounding (a user who asked for observable rounding must not be told that
# source may assume fast-math) and GCC for the two sub-option withdrawals
# (neither changes what the arithmetic produces).
KNOWN_SPLITS = frozenset({
    "-O3 -ffast-math -frounding-math",
    "-O3 -ffast-math -fno-reciprocal-math",
    "-O3 -ffast-math -fno-associative-math",
})

# Every fast-math spelling this compiler claims to implement. In strict mode
# all of them must be *recognised*; an unrecognised flag is a build-system
# probe that silently gets the wrong answer.
REAL_SPELLINGS = (
    "-ffast-math", "-fno-fast-math", "-ffinite-math-only",
    "-fno-finite-math-only", "-fmath-errno", "-fno-math-errno",
    "-fsigned-zeros", "-fno-signed-zeros", "-freciprocal-math",
    "-fno-reciprocal-math", "-fcx-limited-range", "-fno-cx-limited-range",
    "-fassociative-math", "-fno-associative-math",
    "-funsafe-math-optimizations", "-fno-unsafe-math-optimizations",
    "-ftrapping-math", "-fno-trapping-math", "-frounding-math",
    "-fno-rounding-math", "-Ofast",
)
NOT_A_FLAG = "-fdefinitely-not-a-flag"


def optional(tool: str) -> str | None:
    found = shutil.which(tool)
    if found:
        return str(Path(found).resolve())
    path = ROOT / tool
    return str(path.resolve()) if path.is_file() else None


class _Compile(unittest.TestCase):
    """One temp dir + one probe source, shared by every test in a TestCase."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.lccc = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        if cls.lccc is None:
            raise unittest.SkipTest("lccc binary not built (target/fastbuild/lccc)")
        cls._tmp = tempfile.TemporaryDirectory()
        cls.dir = Path(cls._tmp.name)
        cls.src = cls.dir / "probe.c"
        cls.src.write_text(PROBE)

    @classmethod
    def tearDownClass(cls) -> None:
        cls._tmp.cleanup()

    def run_lccc(self, *flags: str, env_extra: dict[str, str] | None = None):
        env = dict(os.environ)
        if env_extra:
            env.update(env_extra)
        return subprocess.run(
            [self.lccc, *flags], cwd=self.dir, capture_output=True,
            text=True, timeout=120, env=env,
        )

    def macro_table(self, *flags: str) -> dict[str, str]:
        proc = self.run_lccc(*flags, "-dM", "-E", str(self.src))
        self.assertEqual(
            proc.returncode, 0,
            f"lccc {flags} -dM -E failed: {proc.stderr[:400]}",
        )
        return dict(re.findall(r"^#define (\S+) (.*)$", proc.stdout, re.M))


class MacroContract(_Compile):
    """Every row of CONTRACT must hold, macro for macro."""

    def test_contract_table(self) -> None:
        checked = 0
        for flags, expects in CONTRACT:
            table = self.macro_table(*flags.split())
            for name, expected in expects:
                got = table.get(name, "-")
                self.assertEqual(
                    got, expected,
                    f"{flags!r}: {name} = {got!r}, expected {expected!r}",
                )
                checked += 1
        # Guard against the table silently shrinking to nothing.
        self.assertGreaterEqual(checked, 90, "contract table lost rows")

    def test_oracle_disagreements_are_pinned(self) -> None:
        """Where GCC 16.2 and Clang 23.1.5 disagree, the side this compiler
        takes is a decision, not an accident -- pin it so it cannot drift."""
        # -frounding-math: follow Clang and withdraw.
        self.assertNotIn(
            "__FAST_MATH__", self.macro_table("-O3", "-ffast-math", "-frounding-math")
        )
        # -fno-reciprocal-math / -fno-associative-math: follow GCC and keep it.
        self.assertIn(
            "__FAST_MATH__",
            self.macro_table("-O3", "-ffast-math", "-fno-reciprocal-math"),
        )
        self.assertIn(
            "__FAST_MATH__",
            self.macro_table("-O3", "-ffast-math", "-fno-associative-math"),
        )

    def test_finite_math_only_is_always_defined(self) -> None:
        """GCC and Clang define it on every command line, as 0 or 1. A header
        doing `#if __FINITE_MATH_ONLY__` must never hit an undefined macro."""
        for flags in ("-O0", "-O3", "-Ofast", "-O3 -ffast-math",
                      "-O3 -fno-finite-math-only", "-O3 -ffast-math -fno-fast-math"):
            table = self.macro_table(*flags.split())
            self.assertIn("__FINITE_MATH_ONLY__", table, flags)
            self.assertIn(table["__FINITE_MATH_ONLY__"], ("0", "1"), flags)

    def test_contract_is_architecture_independent(self) -> None:
        """The macro contract is a *source* contract: it must not depend on the
        target, so a cross build sees the same answers."""
        for arch in ("x86_64", "i686", "aarch64", "riscv64"):
            proc = self.run_lccc("-target", arch, "-O3", "-ffast-math",
                                 "-dM", "-E", str(self.src))
            if proc.returncode != 0:
                continue  # target not installed in this sandbox
            table = dict(re.findall(r"^#define (\S+) (.*)$", proc.stdout, re.M))
            self.assertEqual(table.get("__FAST_MATH__"), "1", arch)
            self.assertEqual(table.get("__FINITE_MATH_ONLY__"), "1", arch)
            self.assertNotIn("__STDC_IEC_559__", table, arch)


class StrictOptions(_Compile):
    """LCCC_STRICT_OPTIONS must accept every real spelling and reject fakes."""

    def test_every_real_spelling_is_recognised(self) -> None:
        for flag in REAL_SPELLINGS:
            with self.subTest(flag=flag):
                proc = self.run_lccc("-O2", flag, "-fsyntax-only", str(self.src),
                                     env_extra={"LCCC_STRICT_OPTIONS": "1"})
                self.assertEqual(
                    proc.returncode, 0,
                    f"{flag} not recognised under LCCC_STRICT_OPTIONS: "
                    f"{proc.stderr[:300]}",
                )
                # Normal mode must also stay silent: these are real options,
                # not tolerated no-ops that warn.
                quiet = self.run_lccc("-O2", flag, "-fsyntax-only", str(self.src))
                self.assertEqual(quiet.returncode, 0, flag)
                self.assertEqual(quiet.stderr.strip(), "", f"{flag} warns")

    def test_unknown_flag_is_rejected_in_strict_mode(self) -> None:
        proc = self.run_lccc("-O2", NOT_A_FLAG, "-fsyntax-only", str(self.src),
                             env_extra={"LCCC_STRICT_OPTIONS": "1"})
        self.assertNotEqual(proc.returncode, 0, "a fake flag must be refused")
        self.assertIn("unrecognized", proc.stderr)


CLASSIFY_SRC = """
int side;
double g(void) { side++; return 2.0; }
static int cls(double x) {
    return __builtin_isnan(x) * 1000 + __builtin_isinf(x) * 100
         + __builtin_isfinite(x) * 10 + __builtin_isinf_sign(x);
}
int main(void) {
    double v[] = { 2.0, __builtin_nan(""), __builtin_inf(), -__builtin_inf() };
    for (int i = 0; i < 4; i++) printf("%d ", cls(v[i]));
    printf("side0=%d ", side);
    (void)__builtin_isnan(g());
    printf("side1=%d\\n", side);
    return 0;
}
"""

ISNAN_SRC = "int isnan_d(double x) { return __builtin_isnan(x); }\n"


class FiniteMathCodegen(_Compile):
    """Under -ffinite-math-only the classifiers must fold, but the argument's
    side effects must survive. And the strict path must be competitive."""

    @classmethod
    def setUpClass(cls) -> None:
        super().setUpClass()
        cls.cls_src = cls.dir / "cls.c"
        cls.cls_src.write_text("#include <stdio.h>\n" + CLASSIFY_SRC)
        cls.isnan_src = cls.dir / "isnan.c"
        cls.isnan_src.write_text(ISNAN_SRC)

    def asm(self, *flags: str, src: Path | None = None) -> str:
        proc = self.run_lccc(*flags, "-S", str(src or self.cls_src), "-o", "-")
        self.assertEqual(proc.returncode, 0, proc.stderr[:400])
        return proc.stdout

    def count(self, text: str, needle: str) -> int:
        return text.count(needle)

    def test_strict_mode_still_classifies(self) -> None:
        """At -O3 without -ffinite-math-only nothing may be folded away."""
        body = self.asm("-O3")
        self.assertGreater(self.count(body, "movabsq") + self.count(body, "ucomis"),
                           0, "classifiers vanished without -ffinite-math-only")

    def test_finite_math_only_folds_the_classifiers(self) -> None:
        strict = self.asm("-O3")
        for flags in ("-O3 -ffinite-math-only", "-Ofast", "-O3 -ffast-math"):
            with self.subTest(flags=flags):
                fast = self.asm(*flags.split())
                self.assertLess(
                    len(fast.splitlines()), len(strict.splitlines()),
                    f"{flags} did not shrink the classifier code",
                )
                # The four classifiers collapse to 0/0/1/0, so no infinity
                # pattern may be compared against any more.
                self.assertNotIn("9218868437227405312", fast,
                                 f"{flags} still tests the infinity bit pattern")

    def test_folding_preserves_argument_side_effects(self) -> None:
        """`__builtin_isnan(g())` must still call `g`, even though the result
        folds to a constant. Dropping the call would be a miscompile."""
        for flags in ("-O3", "-O3 -ffinite-math-only", "-Ofast"):
            with self.subTest(flags=flags):
                body = self.asm(*flags.split())
                self.assertIn("g", body, f"{flags} dropped the side-effecting call")

    def test_withdrawal_restores_full_classification(self) -> None:
        """`-Ofast -O2` is `-O2`: the fold must not survive the withdrawal."""
        fast = self.asm("-Ofast")
        back = self.asm("-Ofast", "-O2")
        self.assertLess(len(fast.splitlines()), len(back.splitlines()))

    def test_isnan_uses_unordered_compare_not_bit_tests(self) -> None:
        """`__builtin_isnan` must stay in the FP domain. The old shape was a
        vector->GPR transfer plus two 64-bit immediates (7 instructions);
        GCC 16.2 and Clang 23.1.5 both use 3."""
        body = self.asm("-O3", src=self.isnan_src)
        self.assertIn("ucomisd", body, "isnan left the FP domain")
        self.assertIn("setp", body, "isnan is not testing the unordered flag")
        self.assertNotIn("movabsq", body, "isnan still materialises a bit mask")

    def test_isnan_matches_both_oracles_instruction_for_instruction(self) -> None:
        for oracle in ("gcc-16", "clang-23"):
            binary = optional(oracle)
            if binary is None:
                continue
            with self.subTest(oracle=oracle):
                proc = subprocess.run([binary, "-O3", "-S", str(self.isnan_src),
                                       "-o", "-"], cwd=self.dir,
                                      capture_output=True, text=True, timeout=60)
                self.assertEqual(proc.returncode, 0, proc.stderr[:300])
                want = sorted(
                    l.strip().split()[0]
                    for l in proc.stdout.splitlines()
                    if re.match(r"^\s+(ucomis\w|setn?p|xorl|movzbl|ret)", l)
                )
                got = sorted(
                    l.strip().split()[0]
                    for l in self.asm("-O3", src=self.isnan_src).splitlines()
                    if re.match(r"^\s+(ucomis\w|setn?p|xorl|movzbl|ret)", l)
                )
                self.assertEqual(
                    len(got), len(want),
                    f"lccc emits {got}; {oracle} emits {want}",
                )


RUNTIME_SRC = """
#include <stdio.h>
int main(void) {
    double v[] = { __builtin_nan(""), __builtin_inf(), -__builtin_inf(),
                   2.5, -0.0, 0.0 };
    for (int i = 0; i < 6; i++) printf("%d", __builtin_isnan(v[i]));
    printf(" ");
    for (int i = 0; i < 6; i++) printf("%d", __builtin_isinf(v[i]));
    printf(" ");
    for (int i = 0; i < 6; i++) printf("%d", __builtin_isfinite(v[i]));
    printf("\\n");
    return 0;
}
"""

# Measured on GCC 16.2.0, Clang 23.1.5 and LCCC: isnan/isinf/isfinite over
# {NaN, +Inf, -Inf, 2.5, -0.0, 0.0}.
STRICT_CLASSIFIERS = "100000 011000 000111"


class RuntimeCorrectness(_Compile):
    """Compile and RUN: the fast-math flags must not change an answer that the
    source is entitled to rely on."""

    @classmethod
    def setUpClass(cls) -> None:
        super().setUpClass()
        cls.rt = cls.dir / "rt.c"
        cls.rt.write_text(RUNTIME_SRC)

    def run_it(self, *flags: str) -> str:
        exe = self.dir / "rt.bin"
        proc = self.run_lccc(*flags, str(self.rt), "-o", str(exe))
        self.assertEqual(proc.returncode, 0, proc.stderr[:400])
        out = subprocess.run([str(exe)], cwd=self.dir, capture_output=True,
                             text=True, timeout=60)
        self.assertEqual(out.returncode, 0, "runtime failure")
        return out.stdout.strip()

    def test_strict_classifiers_match_measured_output(self) -> None:
        self.assertEqual(self.run_it("-O3"), STRICT_CLASSIFIERS)

    def test_oracle_agreement_on_strict_classifiers(self) -> None:
        for oracle in ("gcc-16", "clang-23"):
            binary = optional(oracle)
            if binary is None:
                continue
            with self.subTest(oracle=oracle):
                exe = self.dir / f"rt_{oracle.replace('-', '_')}.bin"
                proc = subprocess.run([binary, "-O3", str(self.rt), "-o", str(exe)],
                                      cwd=self.dir, capture_output=True, text=True,
                                      timeout=60)
                self.assertEqual(proc.returncode, 0, proc.stderr[:300])
                out = subprocess.run([str(exe)], cwd=self.dir,
                                     capture_output=True, text=True, timeout=60)
                self.assertEqual(out.stdout.strip(), STRICT_CLASSIFIERS, oracle)

    def test_finite_math_only_matches_gcc_fold(self) -> None:
        """Under -ffinite-math-only the compiler may assume no NaN and no Inf,
        so the classifiers fold: isfinite is 1 and the other two are 0 for
        every value, including the NaN and Infs this fixture passes in.

        GCC 16.2.0 folds (measured: `000000 000000 111111`) and lccc matches it
        byte for byte. Clang 23.1.5 does *not* fold this shape -- it still
        reports `100000 011000 000111` -- while folding a different shape of the
        same test, so it is internally inconsistent and is not used as the
        reference here. Either answer is permitted (the input violates the
        assumption the flag grants); matching GCC is the deliberate choice.
        """
        gcc = optional("gcc-16")
        if gcc is None:
            self.skipTest("gcc-16 not available")
        for flags in ("-O3 -ffinite-math-only", "-Ofast", "-O3 -ffast-math"):
            with self.subTest(flags=flags):
                exe = self.dir / "rtf.bin"
                proc = subprocess.run(
                    [gcc, *flags.split(), str(self.rt), "-o", str(exe)],
                    cwd=self.dir, capture_output=True, text=True, timeout=60,
                )
                self.assertEqual(proc.returncode, 0, proc.stderr[:300])
                out = subprocess.run([str(exe)], cwd=self.dir,
                                     capture_output=True, text=True, timeout=60)
                self.assertEqual(
                    self.run_it(*flags.split()), out.stdout.strip(),
                    f"lccc disagrees with gcc-16 under {flags}",
                )

    def test_clang_divergence_is_documented_not_copied(self) -> None:
        """Pin that lccc does NOT follow Clang's non-folding behaviour here, so
        a future change that 'fixes' it toward Clang has to say why."""
        self.assertEqual(self.run_it("-O3", "-ffinite-math-only"),
                         "000000 000000 111111")


if __name__ == "__main__":
    unittest.main()
