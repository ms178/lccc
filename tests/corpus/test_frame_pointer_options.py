#!/usr/bin/env python3
"""Explicit frame-pointer requests must survive optimization-level defaults.

GCC 16.2 and Clang 23.1.5 both treat ``-f{,no-}omit-frame-pointer`` as an
explicit user option (opts_set semantics): ``-O`` supplies only the default.
The red-team audit of PR #1110 measured LCCC revoking an earlier explicit
request: ``-fno-omit-frame-pointer -O2`` emitted no saved rbp chain, and
``-fomit-frame-pointer -O0`` kept one.  This module pins the oracle agreement
on the host x86-64 compiler and rejects a wrapper that reproduces the old
order-dependent behavior.
"""
from __future__ import annotations

import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = """\
double fp_probe(double x) {
    volatile double a[4] = {x, x + 1, x + 2, x + 3};
    return a[0] + a[3];
}
"""
LEVELS = ("-O0", "-O1", "-O2", "-O3", "-O4", "-Os", "-Oz", "-Og", "-Ofast")
RBP = re.compile(r"pushq?\s+%rbp|movq\s+%rsp,\s*%rbp")


def _resolve(env: str, *names: str) -> str | None:
    value = os.environ.get(env)
    if value:
        path = Path(value)
        if path.is_file():
            return str(path.resolve())
        found = shutil.which(value)
        if found:
            return str(Path(found).resolve())
    for name in names:
        found = shutil.which(name)
        if found:
            return str(Path(found).resolve())
    candidate = ROOT / "target/fastbuild/lccc"
    if names and names[0] == "lccc" and candidate.is_file():
        return str(candidate.resolve())
    return None


class FramePointerOptions(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.lccc = _resolve("LCCC", "lccc")
        cls.gcc = _resolve("GCC_BIN", "gcc-16")
        cls.clang = _resolve("CLANG", "clang-23", "clang")
        if not cls.lccc or not cls.gcc or not cls.clang:
            raise unittest.SkipTest("pinned lccc/gcc-16/clang-23 toolchain absent")
        cls.tmp = tempfile.TemporaryDirectory(prefix="lccc-fp-options-")
        cls.work = Path(cls.tmp.name)
        cls.source = cls.work / "fp.c"
        cls.source.write_text(FIXTURE)

    @classmethod
    def tearDownClass(cls) -> None:
        if hasattr(cls, "tmp"):
            cls.tmp.cleanup()

    def state(self, compiler: str, flags: list[str]) -> bool:
        output = self.work / "fp.s"
        result = subprocess.run(
            [compiler, *flags, "-S", str(self.source), "-o", str(output)],
            cwd=self.work, capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return bool(RBP.search(output.read_text()))

    def vectors(self) -> list[list[str]]:
        out: list[list[str]] = []
        for level in LEVELS:
            out.append([level])
            out.append(["-fno-omit-frame-pointer", level])
            out.append([level, "-fno-omit-frame-pointer"])
            out.append(["-fomit-frame-pointer", level])
            out.append([level, "-fomit-frame-pointer"])
        out.append(["-fno-omit-frame-pointer", "-O2", "-fomit-frame-pointer"])
        out.append(["-fomit-frame-pointer", "-O0", "-fno-omit-frame-pointer"])
        return out

    def test_probe_is_non_vacuous(self) -> None:
        self.assertFalse(self.state(self.gcc, ["-O2"]))
        self.assertTrue(self.state(self.gcc, ["-fno-omit-frame-pointer", "-O2"]))
        self.assertFalse(self.state(self.gcc, ["-fomit-frame-pointer", "-O0"]))

    def test_lccc_matches_gcc_and_clang_for_every_order(self) -> None:
        for flags in self.vectors():
            with self.subTest(flags=flags):
                expected = self.state(self.gcc, flags)
                self.assertEqual(self.state(self.clang, flags), expected, "clang diverges")
                self.assertEqual(self.state(self.lccc, flags), expected, "lccc diverges")

    def test_order_blind_wrapper_is_rejected(self) -> None:
        wrapper = self.work / "order-blind.py"
        wrapper.write_text(
            "#!" + sys.executable + "\n"
            "import os, sys\n"
            "drop = {'-fomit-frame-pointer', '-fno-omit-frame-pointer'}\n"
            "args = [a for a in sys.argv[1:] if a not in drop]\n"
            f"os.execv({self.lccc!r}, [{self.lccc!r}, *args])\n"
        )
        wrapper.chmod(0o755)
        expected = self.state(self.gcc, ["-fno-omit-frame-pointer", "-O2"])
        with self.assertRaises(AssertionError):
            self.assertEqual(self.state(str(wrapper), ["-fno-omit-frame-pointer", "-O2"]), expected)

    def test_driver_source_pins_the_explicit_origin(self) -> None:
        cli = (ROOT / "src/driver/cli.rs").read_text()
        self.assertIn(
            "self.omit_frame_pointer = self.explicit_omit_frame_pointer.unwrap_or(level > 0);",
            cli,
        )
        self.assertIn("self.explicit_omit_frame_pointer = Some(true);", cli)
        self.assertIn("self.explicit_omit_frame_pointer = Some(false);", cli)


if __name__ == "__main__":
    unittest.main()
