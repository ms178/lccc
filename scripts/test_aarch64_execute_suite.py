"""Execution-harness policy tests: no compiler, assembler or emulator required."""
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts import aarch64_execute_suite as suite


class OraclePolicyTests(unittest.TestCase):
    def run_case(self, *, clang=True, clang_rc=0, gcc=(0, b"ok", b""),
                 lccc=(0, b"ok", b""), reference=(0, b"ok", b"")):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "test.c"
            source.write_text("int main(void) { return 0; }\n")
            tools = suite.Toolchain(root, root / "lccc", root / "as", "gcc",
                                    "qemu", "clang" if clang else None, root, (), 1)
            ok = subprocess.CompletedProcess([], 0, b"", b"")
            clang_compile = subprocess.CompletedProcess([], clang_rc, b"", b"bad flag")
            with patch.object(suite, "compile_reference", side_effect=[ok, clang_compile]), \
                 patch.object(suite, "run", return_value=ok), \
                 patch.object(suite, "execute", side_effect=[gcc, lccc, reference]):
                return suite.test_one(source, tools, root / "work")

    def test_clang_compile_failure_is_harness_error(self):
        result = self.run_case(clang_rc=1)
        self.assertEqual((result.status, result.stage), ("HARNESS-ERROR", "clang-compile"))
        self.assertIn("bad flag", result.detail)
        self.assertEqual(suite.exit_status([result]), 1)

    def test_same_exit_status_different_output_is_actionable(self):
        for reference in [(0, b"wrong\nanswer", b""), (0, b"ok", b"diagnostic")]:
            with self.subTest(reference=reference):
                result = self.run_case(reference=reference)
                self.assertEqual(result.status, "ORACLE-DISAGREE")
                self.assertIn("clang rc=0", result.detail)
                self.assertIn("gcc rc=0", result.detail)
                self.assertIn(repr(reference[1].decode().strip()), result.detail)
                self.assertIn(repr(reference[2].decode().strip()), result.detail)
                self.assertEqual(suite.exit_status([result]), 1)

    def test_lccc_failure_is_not_attributed_to_the_oracle(self):
        result = self.run_case(lccc=(1, b"", b"failure"))
        self.assertEqual((result.status, result.stage), ("FAIL", "execute-vs-gcc"))
        self.assertIn("failure", result.detail)

    def test_matching_execution_and_optional_oracle(self):
        for clang in [True, False]:
            self.assertEqual(self.run_case(clang=clang).status, "PASS")

    def test_detail_is_bounded_and_escaped(self):
        detail = suite.execution_detail("clang", (0, b"x" * 10000, b"a\nb"))
        self.assertLess(len(detail), 500)
        self.assertIn("a\\nb", detail)

    def test_aggregate_status(self):
        for status in ["FAIL", "HARNESS-ERROR", "ORACLE-DISAGREE"]:
            self.assertEqual(suite.exit_status([
                suite.Result("pass.c", "PASS", "execute", 0),
                suite.Result("bad.c", status, "execute", 0),
            ]), 1)
        self.assertEqual(suite.exit_status([
            suite.Result("pass.c", "PASS", "execute", 0),
            suite.Result("skip.c", "SKIP", "gcc-compile", 0),
        ]), 0)


if __name__ == "__main__":
    unittest.main()
