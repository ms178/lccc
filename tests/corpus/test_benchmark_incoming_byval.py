"""Compiler-free contracts for the informational incoming-byval A/B harness."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/benchmark_incoming_byval.py"
SPEC = importlib.util.spec_from_file_location("incoming_byval_benchmark", SCRIPT)
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


class IncomingByvalBenchmarkTest(unittest.TestCase):
    def test_extraction_preserves_roles_and_does_not_include_next_function(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.s"
            source.write_text(".text\nsum_big:\n.cfi_startproc\n"
                              "    movq 8(%rsp), %rax\n"
                              "    ret\n.cfi_endproc\n.size sum_big, .-sum_big\n"
                              "other:\n    ud2\n.size other, .-other\n")
            result = BENCH.extract(source, "original")
            self.assertIn(".type original,@function\noriginal:", result)
            self.assertIn("movq 8(%rsp), %rax", result)
            self.assertIn(".size original, .-original", result)
            self.assertNotIn("ud2", result)

    def test_vector_basic_block_labels_are_private_to_each_variant(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.s"
            source.write_text("cf_wrap_phi:\n    jmp .LBB1\n.LBB1:\n"
                              "    ret\n.size cf_wrap_phi, .-cf_wrap_phi\n")
            old = BENCH.extract(source, "original", "cf_wrap_phi")
            new = BENCH.extract(source, "optimized", "cf_wrap_phi")
            self.assertIn("jmp .LBB1_original", old)
            self.assertIn(".LBB1_original:", old)
            self.assertIn(".LBB1_optimized:", new)
            self.assertNotIn(".LBB1_original", new)

    def test_missing_function_is_not_a_successful_empty_measurement(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.s"
            source.write_text("other:\n    ret\n.size other, .-other\n")
            with self.assertRaises(ValueError):
                BENCH.extract(source, "original")

    def test_invalid_budgets_fail_before_any_native_execution(self):
        for flags in (["--samples", "2"], ["--iterations", "0"],
                      ["--iterations", "1000000001"]):
            result = subprocess.run(
                [sys.executable, str(SCRIPT), "--before", "missing.s",
                 "--after", "missing.s", "--out", "unused.json", *flags],
                capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("samples must be 3..101", result.stderr)


if __name__ == "__main__":
    unittest.main()
