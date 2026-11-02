#!/usr/bin/env python3
"""Focused contracts for the linker harness process wrapper."""

import importlib.util
import pathlib
import os
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

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


class FixtureLibraryEnvironmentTests(unittest.TestCase):
    def test_fixture_precedes_inherited_modern_runtime_paths(self):
        with patch.dict(os.environ, {"LD_LIBRARY_PATH": "/gcc/lib32:/gcc/lib64", "KEEP": "yes"}, clear=True):
            before = dict(os.environ)
            env = HARNESS.fixture_library_env("/fixture")
            self.assertEqual(env["LD_LIBRARY_PATH"], "/fixture:/gcc/lib32:/gcc/lib64")
            self.assertEqual(env["KEEP"], "yes")
            self.assertEqual(dict(os.environ), before)

    def test_absent_or_empty_path_does_not_add_current_directory(self):
        for inherited in ({}, {"LD_LIBRARY_PATH": ""}):
            with self.subTest(inherited=inherited), patch.dict(os.environ, inherited, clear=True):
                self.assertEqual(HARNESS.fixture_library_env("/fixture")["LD_LIBRARY_PATH"], "/fixture")


class CustomFixtureCodeModelTests(unittest.TestCase):
    def test_custom_pic_fixtures_do_not_inherit_driver_defaults(self):
        # Stop deliberately at the C/C++ compilation. Assembly and relocation
        # probes are mocked, not the compile command whose contract we check.
        # This catches regressions even on a default-PIE development machine.
        fixtures = (
            ("_gotpcrelx_relax_test", "-fPIE"),
            ("_gotpcrel_edges_test", "-fPIE"),
            ("_got64_spelling_matrix_test", "-fPIE"),
            ("_absolute_symbols_pic_test", "-fPIE"),
            ("_ie_to_le_forms_test", "-fPIE"),
            ("_movrs_relocations_test", "-fPIE"),
            ("_plain_gotpcrel_test", "-fPIE"),
            ("_so_bound_ifunc_test", "-fPIE"),
            ("_cxx_eh_local_type_test", "-fPIC"),
        )
        for fixture, flag in fixtures:
            with self.subTest(fixture=fixture):
                commands = []

                def fake_sh(command, **kwargs):
                    compiling_main = command[0] in ("unit-cc", "unit-cxx") and any(
                        source in command for source in ("m.c", "main.c", "lt.cpp"))
                    if compiling_main:
                        commands.append(command)
                    output = (b"R_X86_64_GOTPCREL \n" * 7 +
                              b"R_X86_64_IRELATIVE\n" * 2)
                    return SimpleNamespace(returncode=int(compiling_main),
                                           stdout=output, stderr=b"compile stopped")

                args = SimpleNamespace(lccc="/not-executed/lccc", keep=False)
                with patch.object(HARNESS, "CC", "unit-cc"), \
                     patch.object(HARNESS, "CXX", "unit-cxx"), \
                     patch.object(HARNESS, "sh", side_effect=fake_sh), \
                     patch.object(HARNESS, "_assembler_variants",
                                  return_value=[("unit-as", ["unit-as"])]):
                    result = getattr(HARNESS, fixture)(args, [])
                self.assertNotEqual(result.status, "PASS")
                self.assertEqual(len(commands), 1)
                self.assertIn(flag, commands[0])
                self.assertNotIn("-fno-pie", commands[0])


if __name__ == "__main__":
    unittest.main()
