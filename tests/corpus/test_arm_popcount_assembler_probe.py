"""The optional GNU cross-assembler probe must reject broken PATH hits."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "arm_popcount_contract", ROOT / "tests/regression/check_arm_popcount_contract.py"
)
CONTRACT = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = CONTRACT
SPEC.loader.exec_module(CONTRACT)


class AssemblerProbe(unittest.TestCase):
    def test_accepts_executable_assembler(self):
        completed = subprocess.CompletedProcess(["as"], 0, "GNU assembler version", "")
        with patch.object(CONTRACT.subprocess, "run", return_value=completed) as run:
            self.assertTrue(CONTRACT.assembler_is_runnable("aarch64-linux-gnu-as"))
        run.assert_called_once_with(
            ["aarch64-linux-gnu-as", "--version"],
            capture_output=True,
            text=True,
            timeout=10,
        )

    def test_rejects_missing_runtime_library_or_timeout(self):
        for failure in (
            OSError("shared library missing"),
            subprocess.TimeoutExpired(["as", "--version"], 10),
            subprocess.CompletedProcess(["as"], 127, "", "loader failure"),
        ):
            with self.subTest(failure=failure), patch.object(
                CONTRACT.subprocess, "run", side_effect=[failure]
            ):
                self.assertFalse(CONTRACT.assembler_is_runnable("aarch64-linux-gnu-as"))


if __name__ == "__main__":
    unittest.main()
