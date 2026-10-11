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


if __name__ == "__main__":
    unittest.main()
