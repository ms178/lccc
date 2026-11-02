#!/usr/bin/env python3
"""Behavioural tests for the GCC oracle -no-pie wrapper.

`gcc_oracle_nopie_wrapper` (tests/regression/lib/gcc_oracle.sh) writes a shell
wrapper that adds `-no-pie` to -m32 *link* invocations only.  A wrong pass-through
rule would silently change the GCC reference's link mode, so each rule is pinned
here against a fake `gcc` that prints its argv (no toolchain needed).

Run: python3 -m unittest tests/regression/test_gcc_oracle_wrapper.py
"""
from __future__ import annotations

import os
import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LIB = ROOT / "tests" / "regression" / "lib" / "gcc_oracle.sh"


class GccOracleWrapperTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="oracle-wrap-")
        self.tmp = Path(self._tmp.name)
        # A fake "gcc" that prints one argv element per line.  The directory name
        # contains a space and a single quote to exercise the %q quoting.
        odd = self.tmp / "odd dir's"
        odd.mkdir()
        self.fake = odd / "fake-gcc"
        self.fake.write_text('#!/bin/sh\nfor a in "$@"; do printf "%s\\n" "$a"; done\n')
        self.fake.chmod(0o755)
        self.cache = self.tmp / "cache"

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def wrapper(self) -> Path:
        script = (
            f". {shlex.quote(str(LIB))}; "
            "GCC_ORACLE_MAJOR=16; "
            f"gcc_oracle_nopie_wrapper {shlex.quote(str(self.fake))}"
        )
        env = dict(os.environ, XDG_CACHE_HOME=str(self.cache))
        out = subprocess.run(["bash", "-c", script], env=env, capture_output=True,
                             text=True, check=True).stdout.strip()
        return Path(out)

    def argv_seen(self, *args: str) -> list[str]:
        r = subprocess.run([str(self.wrapper()), *args], capture_output=True,
                           text=True, check=True)
        return r.stdout.splitlines()

    def test_m32_link_gets_no_pie_first(self) -> None:
        self.assertEqual(self.argv_seen("-m32", "h.c", "-o", "h"),
                         ["-no-pie", "-m32", "h.c", "-o", "h"])

    def test_non_m32_link_is_untouched(self) -> None:
        self.assertEqual(self.argv_seen("h.c", "-o", "h"), ["h.c", "-o", "h"])

    def test_compile_only_modes_pass_through(self) -> None:
        for flag in ("-c", "-S", "-E", "-M", "-MM", "-fsyntax-only"):
            with self.subTest(flag=flag):
                self.assertEqual(self.argv_seen("-m32", flag, "h.c"),
                                 ["-m32", flag, "h.c"])

    def test_explicit_link_modes_pass_through(self) -> None:
        for flag in ("-r", "-pie", "-no-pie", "-static", "-static-pie", "-shared"):
            with self.subTest(flag=flag):
                self.assertEqual(self.argv_seen("-m32", flag, "h.o", "-o", "x"),
                                 ["-m32", flag, "h.o", "-o", "x"])

    def test_arguments_with_spaces_and_quotes_survive(self) -> None:
        arg = "it's a file.c"
        self.assertEqual(self.argv_seen("-m32", arg, "-o", "x y"),
                         ["-no-pie", "-m32", arg, "-o", "x y"])

    def test_wrapper_is_regenerated_atomically_and_executable(self) -> None:
        w = self.wrapper()
        self.assertTrue(os.access(w, os.X_OK))
        self.assertFalse(list(w.parent.glob("*.tmp.*")), "temporary file leaked")
        self.assertEqual(self.wrapper(), w)  # idempotent


if __name__ == "__main__":
    unittest.main()
