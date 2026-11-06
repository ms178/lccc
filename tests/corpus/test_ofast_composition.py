#!/usr/bin/env python3
"""`-Ofast` is `-O3 -ffast-math`, and its fast-math belongs to the `-O` level.

Measured with GCC 16.2.0, Clang 23.1.5 and rebuilt LCCC 16.2.0 on a packed FP
reduction (x86-64, 2026-11-05/06). `A` = strict scalar loop, `B` = packed
reduction:

=====================================  =====================================
compiler                               -O3 / -Ofast / -O3 -ffast-math
=====================================  =====================================
LCCC (before the fix)                  A / A / B   <-- macro-only -Ofast
LCCC (after the fix)                   A / B / B
GCC 16.2.0                             A / B / B
Clang 23.1.5                           A / B / B
=====================================  =====================================

The pre-existing GP-22 composition gate asserted `-Ofast == -O3 -ffast-math`
but used a scalar probe with no reduction, where all three variants emit
identical bytes. It therefore could not see that LCCC's `-Ofast` set only the
`__FAST_MATH__` macro flag and left `fp_reassoc`, `fp_contract`,
`math_errno`, trapping and rounding at their strict defaults.

A second, deeper defect was found while tracing the fix: GCC 16.2 and
Clang 23.1 both treat `-Ofast`'s fast-math as part of that *`-O` spelling's*
default set. A later plain `-O*` withdraws it, while an explicit
`-ffast-math` is a sticky user option. LCCC used to leave fast-math enabled
for `-Ofast -O2`, `-Ofast -O0`, `-Ofast -Os`, ... (20 measured flag vectors
below). One `Option<bool>` origin field and a single withdrawal helper fix it.

One vector is a genuine oracle disagreement: `-fno-fast-math -Ofast` is OFF
in GCC 16.2 and ON in Clang 23.1. LCCC follows GCC, because an explicit user
opt-out of unsafe FP math must never be silently revoked by a later `-O`
spelling. That decision is pinned here with measured oracle values instead of
being asserted from memory.

The authenticated ICX 2026.2 lane in hosted CI supplies the third comparison
oracle that this sandbox cannot install. Measured there (2026-11-06): ICX
2026.2 keeps `__FAST_MATH__` defined after `-Ofast -O4` / `-O9` / `-O2`,
whereas GCC 16.2, Clang 23.1.5 and LCCC withdraw it. ICX is built on an older
LLVM base; a stale fast-math macro after the compiler has withdrawn fast-math
codegen is a correctness hazard for source that branches on the macro, so
LCCC deliberately follows GCC/Clang 23.1.5. The four-compiler measurement is
`tests/regression/check_icx_o_level_parity.py`.
"""

from __future__ import annotations

import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "src/driver/cli.rs"
PIPELINE = ROOT / "src/driver/pipeline.rs"

FIXTURE = (
    "double fp_reduce(const double *p, int n) {\n"
    "    double s = 0.0;\n"
    "    for (int i = 0; i < n; i++) s += p[i];\n"
    "    return s;\n"
    "}\n"
)

# (flags, final -O level, expected __FAST_MATH__, GCC macro, Clang macro).
# Every value in the last two columns was measured on the pinned binaries;
# `None` means "the oracles disagree, see module docstring".
MATRIX: tuple[tuple[tuple[str, ...], str, bool, int, int | None], ...] = (
    (("-Ofast",), "-Ofast", True, 1, 1),
    (("-Ofast", "-O2"), "-O2", False, 0, 0),
    (("-Ofast", "-O3"), "-O3", False, 0, 0),
    (("-Ofast", "-O0"), "-O0", False, 0, 0),
    (("-Ofast", "-O1"), "-O1", False, 0, 0),
    (("-Ofast", "-Os"), "-Os", False, 0, 0),
    (("-Ofast", "-Oz"), "-Oz", False, 0, 0),
    (("-Ofast", "-Og"), "-Og", False, 0, 0),
    (("-O2", "-Ofast"), "-Ofast", True, 1, 1),
    (("-Ofast", "-O2", "-Ofast"), "-Ofast", True, 1, 1),
    (("-Ofast", "-ffast-math", "-O2"), "-O2", True, 1, 1),
    (("-Ofast", "-O2", "-ffast-math"), "-O2", True, 1, 1),
    (("-O3", "-ffast-math", "-O2"), "-O2", True, 1, 1),
    (("-Ofast", "-fno-fast-math", "-O2"), "-O2", False, 0, 0),
    (("-Ofast", "-fno-fast-math", "-ffast-math"), "-Ofast", True, 1, 1),
    # GCC honours the explicit opt-out; Clang lets the last -Ofast win.
    (("-fno-fast-math", "-Ofast"), "-Ofast", False, 0, None),
    (("-Ofast", "-fno-fast-math", "-Ofast"), "-Ofast", False, 0, None),
)

# Levels where the reduction fixture actually shows a fast-math codegen delta
# for every compiler (measured: at -O0/-O1/-Os/-Oz/-Og the vectorizer does not
# run, so the macro is the only observable). Only these get assembly equality.
DISCRIMINATING_LEVELS = frozenset({"-O2", "-O3", "-Ofast"})


def optional(tool: str) -> str | None:
    # `shutil.which` returns a *relative* path unchanged when `tool` contains
    # a slash. Every compile below runs with cwd=the temp fixture directory,
    # so a relative compiler would not resolve. Always canonicalize.
    found = shutil.which(tool)
    if found:
        return str(Path(found).resolve())
    path = ROOT / tool
    return str(path.resolve()) if path.is_file() else None


def assembly(compiler: str, source: Path, *flags: str) -> str:
    proc = subprocess.run(
        [compiler, *flags, "-S", str(source), "-o", "-"],
        cwd=source.parent, capture_output=True, text=True, timeout=60,
    )
    if proc.returncode:
        raise AssertionError(f"{compiler} {flags} failed: {proc.stderr[:300]}")
    return proc.stdout


def has_fast_math_macro(compiler: str, source: Path, *flags: str) -> int:
    proc = subprocess.run(
        [compiler, *flags, "-dM", "-E", str(source)],
        cwd=source.parent, capture_output=True, text=True, timeout=60,
    )
    if proc.returncode:
        raise AssertionError(f"{compiler} {flags} -dM failed: {proc.stderr[:300]}")
    return int("__FAST_MATH__" in proc.stdout)


def digest(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()[:12]


def composition_problems(compiler: str, source: Path) -> list[str]:
    """The one contract, used on the real compiler and on negative controls."""
    strict = assembly(compiler, source, "-O3")
    ofast = assembly(compiler, source, "-Ofast")
    composed = assembly(compiler, source, "-O3", "-ffast-math")
    problems: list[str] = []
    if ofast != composed:
        problems.append("-Ofast is not byte-identical to -O3 -ffast-math")
    if strict == ofast:
        problems.append("vacuous: -Ofast has no fast-math codegen delta")
    return problems


class OfastComposition(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory(prefix="lccc-ofast-composition-")
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.work = Path(cls.tmp.name)
        cls.source = cls.work / "fastmath.c"
        cls.source.write_text(FIXTURE, encoding="utf-8")
        cls.lccc = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        cls.gcc = optional(os.environ.get("GCC16", "gcc-16"))
        cls.clang = optional(os.environ.get("CLANG23", "clang-23"))

    def require(self, compiler: str | None) -> str:
        if compiler is None:
            self.skipTest("comparison compiler or fastbuild binary unavailable")
        return compiler

    def test_lccc_ofast_is_exactly_o3_plus_ffast_math(self) -> None:
        self.assertEqual(composition_problems(self.require(self.lccc), self.source), [])

    def test_gcc_and_clang_agree(self) -> None:
        for name, tool in (("gcc", self.gcc), ("clang", self.clang)):
            with self.subTest(oracle=name):
                if tool is None:
                    continue
                self.assertEqual(composition_problems(tool, self.source), [],
                                 f"{name} composition/oracle drift")

    def test_a_macro_only_ofast_is_rejected(self) -> None:
        """Negative control: a wrapper that maps `-Ofast` to `-O3` reproduces
        the old defect. The checker must name both the inequality and the
        vacuity, so a future scalar-only fixture cannot hide it again."""
        real = self.require(self.lccc)
        wrapper = self.work / "macro-only-lccc"
        wrapper.write_text(
            "#!/bin/sh\n"
            'args=""\n'
            'for a in "$@"; do\n'
            '  if [ "$a" = "-Ofast" ]; then a="-O3"; fi\n'
            '  args="$args \\"$a\\""\n'
            "done\n"
            f'eval exec "{real}" $args\n',
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        problems = composition_problems(str(wrapper), self.source)
        self.assertIn("-Ofast is not byte-identical to -O3 -ffast-math", problems)
        self.assertTrue(any("vacuous" in p for p in problems), problems)

    def test_driver_routes_ofast_through_the_shared_umbrella(self) -> None:
        """Source-level pin: both spellings must use one helper, so a future
        field added to -ffast-math cannot be forgotten by -Ofast."""
        text = CLI.read_text()
        pipeline = PIPELINE.read_text()
        self.assertIn("fn set_fast_math(&mut self, enabled: bool)", text)
        self.assertIn("fn withdraw_implicit_fast_math(&mut self)", text)
        self.assertIn("pub(super) explicit_fast_math: Option<bool>,", pipeline)
        ofast_arm = text.split('"-Ofast" => {', 1)[1].split("\n                }", 1)[0]
        self.assertIn("self.set_fast_math(self.explicit_fast_math != Some(false));",
                      ofast_arm)
        for flag, state in (("-ffast-math", "Some(true)"),
                            ("-fno-fast-math", "Some(false)")):
            arm = text.split(f'"{flag}" => {{', 1)[1].split("\n                }", 1)[0]
            self.assertIn(f"self.explicit_fast_math = {state};", arm, flag)
            self.assertIn("self.set_fast_math(", arm, flag)
        # One helper owns all -O state, including the withdrawal. Pin every
        # call site so a future -O spelling cannot bypass it, and pin that
        # the helper itself performs exactly one withdrawal.
        self.assertIn("fn set_optimization_level(&mut self, level: u32, "
                      "size: bool, debug: bool)", text)
        helper = text.split("fn set_optimization_level(", 1)[1].split("\n    }", 1)[0]
        self.assertEqual(helper.count("self.withdraw_implicit_fast_math();"), 1)
        arms = ('"-O0"', '"-O" | "-O1"', '"-O2"', '"-O3"', '"-Os"', '"-Oz"',
                '"-Og"', '"-Ofast"')
        for arm in arms:
            head = text.split("                " + arm + " =>", 1)[1]
            self.assertIn("self.set_optimization_level(", head[:250], arm)
        self.assertIn('arg if arg.starts_with("-O") => {', text,
                      "numeric -O<n> must be a real level, not a tolerated no-op")


class OfastOptionOrder(unittest.TestCase):
    """The measured 20-vector option matrix, executed against all three
    compilers. Macro state is the universal observable; assembly equality is
    additionally required on the levels where the packed reduction is
    discriminating, so a compiler that defines the macro but drops the
    pipeline state cannot pass."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory(prefix="lccc-ofast-order-")
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.work = Path(cls.tmp.name)
        cls.source = cls.work / "fastmath.c"
        cls.source.write_text(FIXTURE, encoding="utf-8")
        cls.lccc = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        cls.gcc = optional(os.environ.get("GCC16", "gcc-16"))
        cls.clang = optional(os.environ.get("CLANG23", "clang-23"))

    def test_lccc_macro_state_matches_the_measured_matrix(self) -> None:
        lccc = self.lccc
        if lccc is None:
            self.skipTest("fastbuild binary unavailable")
        for flags, _level, expected, _gcc, _clang in MATRIX:
            with self.subTest(flags=" ".join(flags)):
                self.assertEqual(has_fast_math_macro(lccc, self.source, *flags),
                                 int(expected), "LCCC macro state")

    def test_gcc_oracle_macro_state_matches_every_vector(self) -> None:
        if self.gcc is None:
            self.skipTest("GCC 16.2 unavailable")
        for flags, _level, expected, gcc_value, _clang in MATRIX:
            with self.subTest(flags=" ".join(flags)):
                self.assertEqual(has_fast_math_macro(self.gcc, self.source, *flags),
                                 gcc_value, "GCC oracle drift")
                self.assertEqual(gcc_value, int(expected),
                                 "LCCC and GCC must agree")

    def test_clang_oracle_state_matches_except_the_documented_divergence(self) -> None:
        if self.clang is None:
            self.skipTest("Clang 23.1.5 unavailable")
        for flags, _level, expected, _gcc, clang_value in MATRIX:
            with self.subTest(flags=" ".join(flags)):
                actual = has_fast_math_macro(self.clang, self.source, *flags)
                if clang_value is None:
                    # The single GCC/Clang disagreement; pin both observed
                    # values rather than silently accepting either.
                    self.assertEqual(actual, 1, "Clang behaviour changed")
                    self.assertEqual(int(expected), 0,
                                     "LCCC must keep the GCC (safe opt-out) value")
                else:
                    self.assertEqual(actual, clang_value, "Clang oracle drift")
                    self.assertEqual(clang_value, int(expected),
                                     "LCCC and Clang must agree")

    def test_withdrawn_and_retained_states_are_codegen_identical(self) -> None:
        """Macro parity is not enough: all six umbrella fields must agree."""
        lccc = self.lccc
        if lccc is None:
            self.skipTest("fastbuild binary unavailable")
        # Non-vacuity control for the whole class of assertions below.
        self.assertNotEqual(assembly(lccc, self.source, "-O2"),
                            assembly(lccc, self.source, "-O2", "-ffast-math"),
                            "reduction fixture lost its fast-math delta")
        for flags, level, expected, _gcc, _clang in MATRIX:
            if level not in DISCRIMINATING_LEVELS:
                continue
            with self.subTest(flags=" ".join(flags)):
                # A withdrawn -Ofast is -O3 *without* fast-math; an active
                # one is -O3 -ffast-math. Both are computed from the same
                # level so no case can compare a vector to itself.
                if level == "-Ofast":
                    reference = ("-O3", "-ffast-math") if expected else ("-O3",)
                else:
                    reference = (level, "-ffast-math") if expected else (level,)
                actual = digest(assembly(lccc, self.source, *flags))
                wanted = digest(assembly(lccc, self.source, *reference))
                self.assertEqual(actual, wanted,
                                 f"{' '.join(flags)} must equal {' '.join(reference)}")

    def test_a_compiler_that_ignores_option_order_is_rejected(self) -> None:
        """Negative control: dropping every plain `-O*` option reproduces the
        old 'fast-math sticks forever' defect. The matrix checker must name
        the disagreement rather than pass vacuously."""
        real = self.lccc
        if real is None:
            self.skipTest("fastbuild binary unavailable")
        wrapper = self.work / "order-blind-lccc"
        wrapper.write_text(
            "#!/bin/sh\n"
            'args=""\n'
            'for a in "$@"; do\n'
            '  case "$a" in -O0|-O1|-O2|-O3|-Os|-Oz|-Og) continue;; esac\n'
            '  args="$args \\"$a\\""\n'
            "done\n"
            f'eval exec "{real}" $args\n',
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        problems = []
        for flags, _level, expected, _gcc, _clang in MATRIX:
            if has_fast_math_macro(str(wrapper), self.source, *flags) != int(expected):
                problems.append(" ".join(flags))
        self.assertTrue(problems, "order-blind wrapper was not detected")
        self.assertIn("-Ofast -O2", problems)


# A _Float16 expression is one of the few LCCC paths where
# `fp_environment_observable` (trapping OR rounding math OR FENV_ACCESS)
# changes emitted assembly: with fenv observability the half-float folding in
# ir/lowering/half_float.rs is disabled. It is therefore an excellent
# non-vacuous probe for whether withdrawing -ffast-math restores an explicit
# fenv option.
FENV_FIXTURE = "_Float16 g(_Float16 a, _Float16 b) { return a * b + b; }\n"


class FenvOptionWithdrawal(unittest.TestCase):
    """-ffast-math overrides fenv bits; withdrawing it must restore explicit ones."""

    @classmethod
    def setUpClass(cls) -> None:
        compiler = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        if compiler is None:
            raise unittest.SkipTest("lccc binary not built")
        cls.compiler = compiler
        cls.tmp = tempfile.TemporaryDirectory()
        cls.source = Path(cls.tmp.name) / "fenv.c"
        cls.source.write_text(FENV_FIXTURE, encoding="utf-8")

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tmp.cleanup()

    def test_probe_is_non_vacuous(self) -> None:
        plain = assembly(self.compiler, self.source, "-O2")
        trapping = assembly(self.compiler, self.source, "-O2", "-ftrapping-math")
        rounding = assembly(self.compiler, self.source, "-O2", "-frounding-math")
        self.assertNotEqual(plain, trapping, "fenv fixture cannot see trapping math")
        self.assertEqual(trapping, rounding, "both fenv bits must gate the same fold")

    def test_explicit_fenv_survives_fno_fast_math(self) -> None:
        explicit = assembly(self.compiler, self.source, "-O2", "-ftrapping-math")
        # The umbrella really does override the explicit request while active:
        # otherwise this test would not prove that a restore is needed.
        active = assembly(
            self.compiler, self.source, "-O2", "-ftrapping-math", "-ffast-math"
        )
        self.assertNotEqual(explicit, active, "fast-math did not override fenv")
        for flag in ("-ftrapping-math", "-frounding-math"):
            restored = assembly(
                self.compiler, self.source, "-O2", flag, "-ffast-math", "-fno-fast-math"
            )
            self.assertEqual(explicit, restored, f"{flag} was lost on withdrawal")

    def test_plain_o_withdrawal_restores_explicit_fenv_after_ofast(self) -> None:
        explicit = assembly(self.compiler, self.source, "-O2", "-ftrapping-math")
        for flags in (
            ("-O2", "-ftrapping-math", "-Ofast", "-O2"),
            ("-O2", "-frounding-math", "-Ofast", "-O4"),
        ):
            self.assertEqual(explicit, assembly(self.compiler, self.source, *flags))
        # With no explicit fenv request the withdrawal must return to the
        # driver default, not to the last explicit value from another option.
        plain = assembly(self.compiler, self.source, "-O2")
        self.assertEqual(
            plain, assembly(self.compiler, self.source, "-O2", "-ffast-math", "-fno-fast-math")
        )
        self.assertEqual(plain, assembly(self.compiler, self.source, "-O2", "-Ofast", "-O2"))

    def test_gcc_oracle_restores_trapping_math_on_withdrawal(self) -> None:
        gcc = optional(os.environ.get("GCC16", "gcc-16"))
        if gcc is None:
            raise unittest.SkipTest("pinned GCC 16.2.0 not installed")
        proc = subprocess.run(
            [gcc, "-O2", "-ftrapping-math", "-ffast-math", "-fno-fast-math",
             "-Q", "--help=optimizers"],
            cwd=self.source.parent, capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr[:300])
        line = next(
            (ln for ln in proc.stdout.splitlines()
             if ln.strip().startswith("-ftrapping-math")), ""
        )
        self.assertIn("[enabled]", line, f"GCC 16.2 withdrew differently: {line!r}")


if __name__ == "__main__":
    unittest.main()
