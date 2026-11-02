"""A sidecar which the runner cannot discover must not silently do nothing."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SidecarNames(unittest.TestCase):
    def test_no_double_suffix_sidecars(self):
        folder = ROOT / 'tests/regression'
        bad = sorted(str(p.relative_to(ROOT)) for pattern in ('*.c.env', '*.c.flags') for p in folder.glob(pattern))
        self.assertEqual(bad, [], 'runner consumes NAME.env/NAME.flags, not NAME.c.env')
