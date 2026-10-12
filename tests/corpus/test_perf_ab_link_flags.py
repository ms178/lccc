"""A/B library flags must be symmetric, explicit and reproducible."""
import importlib.util
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("perf_ab_link_contract", ROOT / "scripts/perf_ab.py")
perf = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(perf)


class LinkFlagContract(unittest.TestCase):
    def test_vendor_library_requirement_is_shared_and_order_independent(self):
        for cc in ("gcc", "clang-23", "/tools/icx", "icc"):
            with self.subTest(compiler=cc):
                expected = ["-O2", "-lm"]
                self.assertEqual(perf.matched_link_flags(["-O2"], "lccc", cc), expected)
                self.assertEqual(perf.matched_link_flags(["-O2"], cc, "lccc"), expected)
                self.assertEqual(perf.matched_link_flags(expected, cc, "lccc"), expected)

    def test_local_only_defaults_do_not_change_or_inspect_parent_path(self):
        flags = ["-O2"]
        self.assertEqual(perf.matched_link_flags(flags, "/gcc-work/baseline-lccc", "lccc"), flags)
        self.assertEqual(flags, ["-O2"])

    def test_libraries_follow_source_without_reordering_linker_state(self):
        flags = ["-O2", "-Wl,--as-needed", "-lm", "-Wl,--no-as-needed", "-l", "m"]
        for cc in ("gcc", "lccc"):
            cmd = perf.binary_command(cc, Path("test.c"), "test.bin", flags)
            self.assertEqual(cmd, [cc, "-O2", "test.c", *flags[1:], "-o", "test.bin"])
        self.assertEqual(flags[0], "-O2", "must not mutate caller flags")

    def test_paired_frontend_and_linker_options_keep_their_operands(self):
        flags = ["-x", "c", "-Xclang", "-load", "-Xclang", "plugin.so", "-Xlinker", "--as-needed", "-L", "libs", "-lm"]
        self.assertEqual(perf.binary_command("clang", Path("test.c"), "test.bin", flags),
                         ["clang", "-x", "c", "-Xclang", "-load", "-Xclang", "plugin.so", "test.c",
                          "-Xlinker", "--as-needed", "-L", "libs", "-lm", "-o", "test.bin"])

    def test_missing_paired_argument_never_invokes_the_compiler(self):
        for flag in ("-Xlinker", "-I", "-l", "-x"):
            with self.subTest(flag=flag), patch.object(perf.subprocess, "run") as run:
                ok, error = perf.build_binary("gcc", Path("test.c"), "test.bin", {}, [flag])
                self.assertFalse(ok)
                self.assertIn(flag, error)
                run.assert_not_called()

    def test_assembly_arm_drops_only_linker_inputs(self):
        flags = ["-O2", "-Werror", "-Wl,--as-needed", "-lm", "-Xassembler", "--fatal-warnings"]
        with patch.object(perf.subprocess, "run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, b"", b"")
            ok, _ = perf.build_asm("clang", Path("test.c"), "test.s", {}, flags)
            self.assertTrue(ok)
            self.assertEqual(run.call_args.args[0], ["clang", "-O2", "-Werror", "-Xassembler",
                                                     "--fatal-warnings", "-S", "test.c", "-o", "test.s"])

    def test_frontend_operand_that_looks_like_a_library_stays_frontend(self):
        compile_flags, link_flags = perf.split_driver_flags(["-MF", "-lm", "-lm"])
        self.assertEqual(compile_flags, ["-MF", "-lm"])
        self.assertEqual(link_flags, ["-lm"])

    def test_binary_builder_never_adds_unreported_side_specific_flags(self):
        for cc in ("gcc", "lccc"):
            with self.subTest(compiler=cc), patch.object(perf.subprocess, "run") as run:
                run.return_value = subprocess.CompletedProcess([], 0, b"", b"")
                ok, _ = perf.build_binary(cc, Path("test.c"), "test.bin", {}, ["-O2"])
                self.assertTrue(ok)
                self.assertEqual(run.call_args.args[0], [cc, "-O2", "test.c", "-o", "test.bin"])


if __name__ == "__main__":
    unittest.main()
