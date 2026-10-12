"""The executed oracle must use the explicitly selected frozen compiler."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "godbolt_local_binary_contract", ROOT / "tools/oracle/godbolt_oracle.py")
oracle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(oracle)


class LocalBinaryContract(unittest.TestCase):
    def test_legacy_none_argument_uses_configured_compiler(self):
        failure = subprocess.CompletedProcess([], 1, "", "expected compile failure")
        with patch.object(oracle, "LCCC", "/configured/compiler"), \
                patch.object(oracle.subprocess, "run", return_value=failure) as run:
            result = oracle.local(None, "source.c", "-O2", False, "/unused")
        self.assertEqual(run.call_args.args[0][0], "/configured/compiler")
        self.assertFalse(result["ok"])

    def test_both_compile_legs_use_requested_binary(self):
        for execute in (False, True):
            with self.subTest(execute=execute), tempfile.TemporaryDirectory() as tmp:
                calls = []

                def run(argv, **kwargs):
                    calls.append(argv)
                    if "-S" in argv:
                        Path(argv[argv.index("-o") + 1]).write_text("main:\n    ret\n")
                    return subprocess.CompletedProcess(argv, 0, "witness\n", "")

                with patch.object(oracle, "LCCC", "/wrong/current/compiler"), \
                        patch.object(oracle.subprocess, "run", side_effect=run):
                    result = oracle.local("/chosen/frozen/compiler", "source.c",
                                          "-O2 -fwrapv", execute, tmp)
                self.assertTrue(result["ok"])
                self.assertEqual(calls[0][0], "/chosen/frozen/compiler")
                self.assertEqual(calls[0][1:3], ["-O2", "-fwrapv"])
                if execute:
                    self.assertEqual(calls[1][0], "/chosen/frozen/compiler")
                    self.assertEqual(calls[2], [str(Path(tmp) / "p.bin")])
                    self.assertEqual(result["exit"], 0)
                    self.assertEqual(result["stdout"], "witness\n")
                self.assertEqual(len(calls), 3 if execute else 1)

    def test_link_flags_follow_source_and_quoted_flags_stay_intact(self):
        with tempfile.TemporaryDirectory() as tmp:
            calls = []
            def run(argv, **kwargs):
                calls.append(argv)
                if "-S" in argv:
                    Path(argv[argv.index("-o") + 1]).write_text("main:\n    ret\n")
                return subprocess.CompletedProcess(argv, 0, "", "")
            with patch.object(oracle.subprocess, "run", side_effect=run):
                result = oracle.local("cc", "source.c",
                                      '-O2 -I"path with spaces" -Wl,--as-needed -lm', True, tmp)
            self.assertTrue(result["ok"])
            self.assertIn("-Ipath with spaces", calls[0])
            self.assertNotIn("-lm", calls[0])
            self.assertNotIn("-Wl,--as-needed", calls[0])
            self.assertLess(calls[1].index("source.c"), calls[1].index("-lm"))

    def test_malformed_flags_fail_without_invoking_compiler(self):
        for flags in ('-I', '-I"unterminated'):
            with self.subTest(flags=flags), patch.object(oracle.subprocess, "run") as run:
                result = oracle.local("cc", "source.c", flags, False, "/unused")
                self.assertFalse(result["ok"])
                run.assert_not_called()

    def test_local_ignored_target_is_not_success(self):
        response = subprocess.CompletedProcess([], 0, "", "warning: argument unused during compilation: '-march=wrong'")
        with patch.object(oracle.subprocess, "run", return_value=response):
            result = oracle.local("cc", "source.c", "-march=wrong", False, "/unused")
        self.assertFalse(result["ok"])
        self.assertIn("not honored", result["reason"])


if __name__ == "__main__":
    unittest.main()
