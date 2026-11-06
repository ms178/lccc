#!/usr/bin/env python3
"""Numeric `-O<n>` is a real optimization level, not a tolerated no-op.

Measured on x86-64 with GCC 16.2.0, Clang 23.1.5 and rebuilt LCCC 16.2.0
(packed FP reduction, assembly SHA-256 of `-S` output):

===========================  ===============================================
spelling                     behaviour
===========================  ===============================================
`-O4` .. `-O2147483647`      GCC/Clang clamp to their own `-O3`; LCCC used
                             to fall through to the generic
                             tolerated-unknown handler and emit the
                             *unoptimized* default with only a warning.
`-O2147483648` and above     Clang rejects (`invalid integral value`);
                             GCC wraps `2^32` to `-O0`. LCCC rejects:
                             no arithmetic-overflow artifact, no silent
                             performance cliff.
`-O-1`, `-O+2`, `-O2x`       GCC and Clang reject; LCCC used to tolerate
                             them silently.
`-Ofast -O4`                 all three compilers withdraw `-Ofast`'s
                             implicit fast-math; LCCC used to keep it.
===========================  ===============================================

`-O4` still appears in real build systems (legacy Makefiles, autoconf
probes). Silently producing unoptimized code there is a measured
"cycles left behind" defect, so this module pins the level, the macro, the
exit status, the absence of a diagnostic, and the oracle agreement.
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

FIXTURE = (
    "double fp_reduce(const double *p, int n) {\n"
    "    double s = 0.0;\n"
    "    for (int i = 0; i < n; i++) s += p[i];\n"
    "    return s;\n"
    "}\n"
)

# Every one of these must compile and be byte-identical to `-O3`.
ACCEPTED = (
    "-O3", "-O4", "-O5", "-O9", "-O10", "-O999", "-O2147483647",
    "-O0000000000000000000000004",
)
# Every one of these must be a hard error for LCCC and Clang. GCC diverges on
# the two wraparound values; that divergence is pinned separately rather than
# being silently accepted.
REJECTED = (
    "-O-1", "-O+2", "-O2x", "-O2147483648", "-O4294967295",
    "-O4294967296", "-O999999999999999999999999", "-Ofast=3",
)
GCC_WRAPS = frozenset({"-O2147483648", "-O4294967296"})
# Measured oracle divergences on malformed spellings. Clang 23.1 accepts
# `-O-1` (as -O3) and parses `-Ofast=3` as `-Ofast`; GCC 16.2 rejects both.
# LCCC follows GCC's fail-closed behaviour for malformed levels.
CLANG_ACCEPTS = frozenset({"-O-1", "-Ofast=3"})


def optional(tool: str) -> str | None:
    found = shutil.which(tool)
    if found:
        return str(Path(found).resolve())
    path = ROOT / tool
    return str(path.resolve()) if path.is_file() else None


def compile_asm(compiler: str, source: Path, *flags: str) -> subprocess.CompletedProcess:
    out = source.parent / "out.s"
    out.unlink(missing_ok=True)
    proc = subprocess.run(
        [compiler, *flags, "-S", str(source), "-o", str(out)],
        cwd=source.parent, capture_output=True, text=True, timeout=60,
    )
    digest = ""
    if out.exists():
        digest = hashlib.sha256(out.read_bytes()).hexdigest()
    return subprocess.CompletedProcess(proc.args, proc.returncode,
                                       stdout=digest, stderr=proc.stderr)


def macros(compiler: str, source: Path, *flags: str) -> str:
    return subprocess.run(
        [compiler, *flags, "-dM", "-E", str(source)],
        cwd=source.parent, capture_output=True, text=True, timeout=60,
    ).stdout


class NumericOptimizationLevels(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory(prefix="lccc-numeric-o-")
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.work = Path(cls.tmp.name)
        cls.source = cls.work / "reduce.c"
        cls.source.write_text(FIXTURE, encoding="utf-8")
        cls.lccc = optional(os.environ.get("CCC", "target/fastbuild/lccc"))
        cls.gcc = optional(os.environ.get("GCC16", "gcc-16"))
        cls.clang = optional(os.environ.get("CLANG23", "clang-23"))

    def require(self, tool: str | None) -> str:
        if tool is None:
            self.skipTest("pinned comparison compiler or fastbuild binary unavailable")
        return tool

    def test_lccc_numeric_levels_are_byte_identical_to_o3(self) -> None:
        lccc = self.require(self.lccc)
        reference = compile_asm(lccc, self.source, "-O3")
        self.assertEqual(reference.returncode, 0)
        self.assertTrue(reference.stdout, "-O3 produced no assembly")
        for flag in ACCEPTED:
            with self.subTest(flag=flag):
                got = compile_asm(lccc, self.source, flag)
                self.assertEqual(got.returncode, 0, got.stderr[:300])
                self.assertEqual(got.stdout, reference.stdout,
                                 f"{flag} did not produce -O3 code")
                self.assertEqual(got.stderr, "",
                                 f"{flag} must not emit a tolerated-unknown warning")

    def test_numeric_levels_define_optimize_not_optimize_size(self) -> None:
        lccc = self.require(self.lccc)
        for flag in ACCEPTED:
            with self.subTest(flag=flag):
                text = macros(lccc, self.source, flag)
                self.assertIn("#define __OPTIMIZE__ 1", text)
                self.assertNotIn("#define __OPTIMIZE_SIZE__", text)
                self.assertNotIn("__FAST_MATH__", text)
        self.assertNotIn("#define __OPTIMIZE__", macros(lccc, self.source, "-O0"))

    def test_gcc_and_clang_clamp_numeric_levels_to_their_own_o3(self) -> None:
        for name, tool in (("gcc", self.gcc), ("clang", self.clang)):
            if tool is None:
                continue
            with self.subTest(oracle=name):
                reference = compile_asm(tool, self.source, "-O3")
                self.assertEqual(reference.returncode, 0)
                for flag in ACCEPTED:
                    with self.subTest(oracle=name, flag=flag):
                        got = compile_asm(tool, self.source, flag)
                        self.assertEqual(got.returncode, 0, got.stderr[:300])
                        self.assertEqual(got.stdout, reference.stdout,
                                         f"{name} {flag} != {name} -O3")

    def test_malformed_and_out_of_range_levels_are_errors(self) -> None:
        lccc = self.require(self.lccc)
        for flag in REJECTED:
            with self.subTest(flag=flag):
                got = compile_asm(lccc, self.source, flag)
                self.assertNotEqual(got.returncode, 0, f"{flag} was accepted")
                self.assertIn("unrecognized command-line option", got.stderr)
        if self.clang:
            for flag in REJECTED:
                with self.subTest(oracle="clang", flag=flag):
                    got = compile_asm(self.clang, self.source, flag)
                    if flag in CLANG_ACCEPTS:
                        self.assertEqual(got.returncode, 0,
                                         "Clang divergence changed")
                    else:
                        self.assertNotEqual(got.returncode, 0)
        if self.gcc:
            for flag in REJECTED:
                with self.subTest(oracle="gcc", flag=flag):
                    got = compile_asm(self.gcc, self.source, flag)
                    if flag in GCC_WRAPS:
                        self.assertEqual(got.returncode, 0,
                                         "GCC wraparound behaviour changed")
                    else:
                        self.assertNotEqual(got.returncode, 0)

    def test_ofast_then_numeric_withdraws_fast_math(self) -> None:
        """The numeric level must go through the same shared helper as -O2."""
        lccc = self.require(self.lccc)
        strict = compile_asm(lccc, self.source, "-O3")
        fast = compile_asm(lccc, self.source, "-Ofast")
        self.assertNotEqual(strict.stdout, fast.stdout,
                            "reduction fixture lost its fast-math delta")
        for flag in ("-O0", "-O1", "-O2", "-O3", "-O4", "-O9", "-Os", "-Oz",
                     "-Og"):
            with self.subTest(flag=flag):
                got = compile_asm(lccc, self.source, "-Ofast", flag)
                self.assertEqual(got.returncode, 0, got.stderr[:300])
                self.assertNotIn("__FAST_MATH__",
                                 macros(lccc, self.source, "-Ofast", flag))
        self.assertEqual(
            compile_asm(lccc, self.source, "-Ofast", "-O4").stdout,
            strict.stdout,
            "-Ofast -O4 must be plain -O3, not -Ofast")

    def test_a_compiler_that_drops_numeric_levels_is_rejected(self) -> None:
        """Negative control: a wrapper that strips `-O<n>` reproduces the old
        silent-unoptimized-code defect. The equality checker must catch it."""
        real = self.require(self.lccc)
        wrapper = self.work / "drop-numeric-o"
        wrapper.write_text(
            "#!/bin/sh\n"
            'args=""\n'
            'for a in "$@"; do\n'
            '  case "$a" in -O[0-9]*) continue;; esac\n'
            '  args="$args \\"$a\\""\n'
            "done\n"
            f'eval exec "{real}" $args\n',
            encoding="utf-8",
        )
        wrapper.chmod(0o755)
        reference = compile_asm(real, self.source, "-O3").stdout
        drift = [flag for flag in ACCEPTED
                 if compile_asm(str(wrapper), self.source, flag).stdout != reference]
        self.assertTrue(drift, "the silent performance cliff was not detected")
        self.assertIn("-O4", drift)

    def test_strict_options_does_not_reimplement_a_real_level(self) -> None:
        """LCCC_STRICT_OPTIONS=1 must keep -O4 implemented and keep -O2x
        rejected; the numeric arm is a real feature, not a tolerance."""
        lccc = self.require(self.lccc)
        env = dict(os.environ, LCCC_STRICT_OPTIONS="1")
        for flag, want_rc in (("-O4", 0), ("-O2x", 1)):
            with self.subTest(flag=flag):
                proc = subprocess.run(
                    [lccc, flag, "-S", str(self.source), "-o", "-"],
                    cwd=self.source.parent, capture_output=True, text=True,
                    timeout=60, env=env,
                )
                self.assertEqual(proc.returncode == 0, want_rc == 0,
                                 f"{flag}: {proc.stderr[:300]}")

    def test_all_four_arch_drivers_agree_on_the_new_levels(self) -> None:
        """The driver option table is architecture-independent, but a
        per-target CLI could diverge. Measured on the rebuilt fastbuild tree:
        lccc, lccc-i686, lccc-arm and lccc-riscv all satisfy the same level
        invariants and macro states. Each is a host executable emitting a
        different target's assembly, so this exercises four back ends."""
        drivers = ("lccc", "lccc-i686", "lccc-arm", "lccc-riscv")
        paths = [ROOT / "target" / "fastbuild" / name for name in drivers]
        if not all(path.is_file() for path in paths):
            self.skipTest("one or more fastbuild arch drivers were not built")
        vectors = {
            "O3": ("-O3",),
            "O2": ("-O2",),
            "O4": ("-O4",),
            "Ofast": ("-Ofast",),
            "O3FF": ("-O3", "-ffast-math"),
            "OfastO2": ("-Ofast", "-O2"),
            "OfastO4": ("-Ofast", "-O4"),
        }
        macro_states = {"O4": 0, "Ofast": 1, "OfastO2": 0,
                        "OfastO4": 0, "O3FF": 1}
        for path in paths:
            with self.subTest(driver=path.name):
                codegen = {name: compile_asm(str(path), self.source, *flags).stdout
                           for name, flags in vectors.items()}
                self.assertEqual(codegen["O4"], codegen["O3"], "numeric clamp")
                self.assertEqual(codegen["Ofast"], codegen["O3FF"], "umbrella")
                self.assertEqual(codegen["OfastO2"], codegen["O2"], "withdrawal")
                self.assertEqual(codegen["OfastO4"], codegen["O3"], "-Ofast -O4")
                for name, expected in macro_states.items():
                    with self.subTest(driver=path.name, vector=name):
                        self.assertEqual(
                            int("__FAST_MATH__" in macros(
                                str(path), self.source, *vectors[name])),
                            expected)

    def test_the_arch_driver_set_is_pinned(self) -> None:
        """A new target must be added to BOTH parity sweeps deliberately.
        Read the executable ledger's own driver set as text (import-path
        independent under `unittest discover`), not a literal copy of this
        module's tuple (which would be vacuous)."""
        other = (ROOT / "tests/corpus/test_option_effect_ledger.py").read_text()
        self.assertIn(
            'DRIVERS = ("lccc", "lccc-i686", "lccc-arm", "lccc-riscv")',
            other,
            "the ledger sweep's arch set changed; update this sweep too")

    def test_numeric_level_uses_the_shared_optimization_helper(self) -> None:
        """Source pin: a second state-assignment block would reintroduce the
        drift that made -O4 a no-op."""
        text = CLI.read_text()
        self.assertIn('arg if arg.starts_with("-O") => {', text)
        arm = text.split('arg if arg.starts_with("-O") => {', 1)[1]
        arm = arm.split("\n                }", 1)[0]
        self.assertIn("self.set_optimization_level(", arm)
        self.assertIn("i32::MAX", arm, "out-of-range guard must be explicit")
        self.assertIn("fn set_optimization_level(&mut self, level: u32,", text)


if __name__ == "__main__":
    unittest.main()
