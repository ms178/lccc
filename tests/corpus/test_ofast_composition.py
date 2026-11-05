#!/usr/bin/env python3
"""`-Ofast` is `-O3 -ffast-math`, not an `-O3` alias that defines a macro.

Measured with GCC 16.2.0, Clang 23.1.5 and the rebuilt LCCC 16.2.0 on a
packed FP reduction (x86-64, 2026-11-05):

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
`math_errno`, trapping and rounding at their strict defaults. On the packed
reduction that is an 11-instruction scalar loop versus a 63-instruction
vectorized loop.

These tests enforce both equality AND non-vacuity for LCCC, GCC and Clang, and
include a negative control: a wrapper compiler that translates `-Ofast` to
`-O3` (the old behaviour) must be rejected by the same checker. The
authenticated ICX 2026.2 lane in hosted CI supplies the third comparison
oracle that this sandbox cannot install.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CLI = ROOT / "src/driver/cli.rs"

FIXTURE = (
    "double fp_reduce(const double *p, int n) {\n"
    "    double s = 0.0;\n"
    "    for (int i = 0; i < n; i++) s += p[i];\n"
    "    return s;\n"
    "}\n"
)


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
        """Source-level pin: the two assignments must be one shared helper,
        so a future flag added to -ffast-math cannot be forgotten by -Ofast."""
        text = CLI.read_text()
        self.assertIn("fn set_fast_math(&mut self, enabled: bool)", text)
        self.assertIn('"-Ofast" => {', text)
        ofast_arm = text.split('"-Ofast" => {', 1)[1].split("}", 1)[0]
        self.assertIn("self.set_fast_math(true);", ofast_arm)
        self.assertIn('"-ffast-math" => self.set_fast_math(true)', text)
        self.assertIn('"-fno-fast-math" => self.set_fast_math(false)', text)


if __name__ == "__main__":
    unittest.main()
