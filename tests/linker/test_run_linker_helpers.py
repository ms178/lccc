#!/usr/bin/env python3
"""Focused contracts for the linker harness process wrapper."""

import importlib.util
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("run_linker_tests", HERE / "run_linker_tests.py")
HARNESS = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(HARNESS)


class ShellWrapperTests(unittest.TestCase):
    def test_empty_command_is_rejected_explicitly(self):
        with self.assertRaisesRegex(ValueError, "non-empty command"):
            HARNESS.sh([])

    def test_missing_executable_names_the_executable(self):
        missing = "/definitely/not/an/lccc-tool"
        with self.assertRaisesRegex(RuntimeError, r"required tool not found: .*lccc-tool"):
            HARNESS.sh([missing])

    def test_missing_cwd_is_not_misreported_as_a_missing_tool(self):
        with tempfile.TemporaryDirectory() as parent:
            missing = str(pathlib.Path(parent) / "gone")
            with self.assertRaisesRegex(RuntimeError, r"working directory not found: .*gone"):
                HARNESS.sh(["true"], cwd=missing)


if __name__ == "__main__":
    unittest.main()
