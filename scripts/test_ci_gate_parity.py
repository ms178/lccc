#!/usr/bin/env python3
"""Mutation tests: distinct asm-diff mode/corpus gates must stay hosted."""
from __future__ import annotations

from contextlib import redirect_stderr
from io import StringIO
import unittest

import check_ci_gate_parity as parity


class AsmDiffParityTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.local = parity.LOCAL.read_text()
        cls.hosted = "\n".join(
            parity.run_script_bodies(p)
            for p in sorted(parity.WORKFLOWS.glob("*.yml"))
        )
        # Retain the literal YAML run lines (including shell quoting) so
        # each negative mutation changes exactly one hosted invocation.
        cls.x64 = next(
            line.strip() for line in cls.hosted.splitlines()
            if line.strip().startswith("python3 scripts/asmdiff.py")
            and "merged-pr629-followup.casefile" in line
        )
        cls.i686 = next(
            line.strip() for line in cls.hosted.splitlines()
            if line.strip().startswith("python3 scripts/asmdiff.py") and "--32" in line
        )

    def assert_rejected(self, hosted: str) -> None:
        with redirect_stderr(StringIO()):
            self.assertEqual(parity.check_asmdiff_gate_parity(self.local, hosted), 1)

    def test_current_gates_are_distinct_and_present(self) -> None:
        self.assertIn(self.i686, self.hosted)
        self.assertIn(self.x64, self.hosted)
        self.assertEqual(parity.check_asmdiff_gate_parity(self.local, self.hosted), 0)

    def test_i686_gate_cannot_stand_in_for_missing_x64_corpus(self) -> None:
        hosted = self.hosted.replace(self.x64, "# x64 differential removed", 1)
        self.assertIn(self.i686, hosted)
        self.assert_rejected(hosted)

    def test_x64_gate_cannot_stand_in_for_missing_i686_mode(self) -> None:
        hosted = self.hosted.replace(self.i686, "# i686 differential removed", 1)
        self.assertIn(self.x64, hosted)
        self.assert_rejected(hosted)

    def test_wrong_mode_compiler_corpus_or_jobs_cannot_satisfy_x64_gate(self) -> None:
        replacements = (
            self.x64.replace("--jobs 2", "--jobs 128", 1),
            self.x64.replace("--lccc target/fastbuild/lccc-x86", "--lccc target/fastbuild/lccc-i686", 1),
            self.x64.replace("merged-pr629-followup.casefile", "other.casefile", 1),
            self.x64.replace("python3 scripts/asmdiff.py", "echo python3 scripts/asmdiff.py", 1),
            self.x64.replace("--as \"$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin/as\"", "--as as", 1),
        )
        for replacement in replacements:
            with self.subTest(replacement=replacement):
                self.assertNotEqual(replacement, self.x64)
                self.assert_rejected(self.hosted.replace(self.x64, replacement, 1))

    def test_installer_is_not_optional_when_hosted_oracle_is_pinned(self) -> None:
        needle = "bash scripts/ensure_gas_247.sh x86_64-linux-gnu"
        self.assertIn(needle, self.hosted)
        self.assert_rejected(self.hosted.replace(needle, "echo installer removed", 1))


if __name__ == "__main__":
    unittest.main()
