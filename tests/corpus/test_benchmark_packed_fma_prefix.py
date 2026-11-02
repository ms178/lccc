"""Compiler-free contracts for the informational packed-FMA A/B harness."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/benchmark_packed_fma_prefix.py"
SPEC = importlib.util.spec_from_file_location("packed_fma_benchmark", SCRIPT)
BENCH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCH)


class PackedFmaBenchmarkTest(unittest.TestCase):
    def test_extraction_preserves_roles_and_does_not_include_next_function(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "input.s"
            source.write_text(".text\nfma_chain:\n.cfi_startproc\n"
                              "    vfmadd132ps %ymm0, %ymm2, %ymm1\n"
                              "    ret\n.cfi_endproc\n.size fma_chain, .-fma_chain\n"
                              "other:\n    ud2\n.size other, .-other\n")
            result = BENCH.extract(source, "original")
            self.assertIn(".globl original\noriginal:", result)
            self.assertIn("vfmadd132ps %ymm0, %ymm2, %ymm1", result)
            self.assertIn(".size original, .-original", result)
            self.assertNotIn("ud2", result)

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
            self.assertIn("need >=3 samples", result.stderr)


if __name__ == "__main__":
    unittest.main()
