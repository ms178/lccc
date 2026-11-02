#!/usr/bin/env python3
"""Assembler selection is independent of a C driver's configured as path.

These are controller tests, not linker-runtime evidence. The strict real suite
separately executes the assembled fixtures and checks relocation/ELF bytes.
"""
import importlib.util
from pathlib import Path
import subprocess
from types import SimpleNamespace
import unittest
from unittest.mock import patch

PATH = Path(__file__).with_name("run_linker_tests.py")
SPEC = importlib.util.spec_from_file_location("linker_fixture_suite", PATH)
SUITE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SUITE)


class AssemblerSelectionTests(unittest.TestCase):
    def invoke(self, assembly_error=None, fail_script=False):
        commands = []

        def run(command, **kwargs):
            commands.append(command)
            if command[0] == "readelf":
                return subprocess.CompletedProcess(command, 0,
                    b"R_X86_64_GOTPCREL \n" * 7, b"")
            if any(arg.endswith(".s") for arg in command):
                # Model Ubuntu's GCC: its configured absolute as path wins
                # over PATH. Neither -Wa nor a new PATH fixes that driver.
                if command[0] == SUITE.CC:
                    return subprocess.CompletedProcess(command, 1, b"",
                        b"pg.s: Error: no such instruction: `movrs'\n")
                if assembly_error and (not fail_script or "sc.s" in command):
                    return subprocess.CompletedProcess(command, 1, b"", assembly_error)
                return subprocess.CompletedProcess(command, 0, b"", b"")
            # Stop after both actual assembly invocations; no runtime/ELF
            # result is fabricated by this controller test.
            return subprocess.CompletedProcess(command, 73, b"", b"C fixture sentinel")

        with patch.object(SUITE, "sh", side_effect=run):
            result = SUITE._plain_gotpcrel_test(SimpleNamespace(keep=False), [])
        return result, commands

    def test_pinned_path_assembler_not_driver_private_assembler(self):
        result, commands = self.invoke()
        self.assertEqual(result.status, "FAIL")
        self.assertIn("C fixture sentinel", result.detail)
        assembly = [cmd for cmd in commands if any(a.endswith(".s") for a in cmd)]
        self.assertEqual(assembly, [
            ["as", "--64", "-mrelax-relocations=no", "pg.s", "-o", "pg.o"],
            ["as", "--64", "-mrelax-relocations=no", "sc.s", "-o", "sc.o"],
        ])

    def test_assembly_errors_are_not_accepted(self):
        for fail_script in (False, True):
            with self.subTest(fail_script=fail_script):
                result, commands = self.invoke(b"bad assembly input", fail_script)
                self.assertEqual(result.status, "FAIL")
                self.assertIn("bad assembly input", result.detail)
                self.assertFalse(any("m.c" in cmd for cmd in commands))


if __name__ == "__main__":
    unittest.main()
