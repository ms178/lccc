#!/usr/bin/env python3
"""Compiler-independent measurement-harness failure/provenance tests."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from measure_volatile_reductions import (
    checked_run,
    positive,
    provenance,
    sha256,
    summary,
)


class MeasurementTests(unittest.TestCase):
    def run_python(self, code, timeout=5):
        return checked_run(
            [sys.executable, "-c", code], env=os.environ.copy(), timeout=timeout
        )

    def test_positive(self):
        self.assertEqual(positive("9"), 9)
        for value in ("0", "-1"):
            with self.assertRaises(argparse.ArgumentTypeError):
                positive(value)

    def test_child_failures(self):
        with self.assertRaisesRegex(
            RuntimeError, r"(?s)child failed \(3\).*deliberate"
        ):
            self.run_python(
                "import sys; print('deliberate', file=sys.stderr); sys.exit(3)"
            )
        with self.assertRaises(subprocess.TimeoutExpired):
            self.run_python("import time; time.sleep(10)", timeout=0.03)
        with self.assertRaises(RuntimeError):
            self.run_python("import sys; print('warning: bad', file=sys.stderr)")
        self.assertEqual(self.run_python("print('ok')").stdout, b"ok\n")

    def test_hash_and_dispersion(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "file"
            path.write_bytes(b"abc")
            self.assertEqual(
                sha256(path),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            )
        self.assertEqual(
            summary([1, 2, 3]), {"median": 2, "mad": 1, "min": 1, "max": 3}
        )

    def test_provenance_and_wrong_reference_version(self):
        result = provenance({"baseline": Path(sys.executable)}, os.environ.copy())
        self.assertIn("sha256", result["tools"]["baseline"])
        for name in ("gcc", "clang", "icx"):
            with self.assertRaises(ValueError):
                provenance({name: Path(sys.executable)}, os.environ.copy())


if __name__ == "__main__":
    unittest.main()
