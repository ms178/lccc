"""Reject unspecified representation bytes as cross-vendor correctness data."""
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class DefinedOracleTests(unittest.TestCase):
    def test_generator_drift_and_full_active_union_members(self):
        with tempfile.TemporaryDirectory() as td:
            output = Path(td) / 'matrix.c'
            subprocess.run([sys.executable, str(REPO / 'tests/regression/gen_string_init_matrix.py'),
                            str(output)], check=True)
            self.assertEqual(output.read_bytes(), (REPO / 'tests/regression/array_string_init_matrix.c').read_bytes())
            lines = [line for line in output.read_text().splitlines() if 'static void t_' in line and '_union' in line]
            self.assertEqual(len(lines), 16)
            self.assertTrue(all('v.s, sizeof v.s' in line for line in lines))

    def test_out_of_range_policy_selftest_has_explicit_scope(self):
        text = (REPO / 'tests/regression/float_cast_saturate.env').read_text()
        self.assertIn('LCCC_NO_COMPARE=1', text)
        self.assertIn('6.3.1.4', text)
        self.assertIn('2147483520.0f', (REPO / 'tests/regression/float_cast_defined_boundaries.c').read_text())


if __name__ == '__main__': unittest.main()
