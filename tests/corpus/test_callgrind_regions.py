"""Client-controlled regions: STOP can reset summary, not file cost totals."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import callgrind_ab as cg

class RegionTotals(unittest.TestCase):
    def parse(self, text):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / 'region.cg'
            p.write_text(text)
            return cg.parse_region_totals(p)

    def header(self):
        return 'events: Ir Dr Dw I1mr D1mr D1mw ILmr DLmr DLmw Bc Bcm Bi Bim\n'

    def test_explicit_totals_not_reset_summary(self):
        result = self.parse(self.header() + 'summary: 18446744073709551605\ntotals: 101 2 3 4 5 6 7 8 9 10 11 12 13\n')
        self.assertEqual(result['Ir'], 101)
        self.assertEqual(result['Bim'], 13)

    def test_documented_trailing_zero_elision(self):
        result = self.parse(self.header() + 'totals: 101\n')
        self.assertEqual(result['Ir'], 101)
        self.assertEqual(result['Bim'], 0)

    def test_invalid_totals_fail_closed(self):
        for text in ['', 'totals:\n', 'totals: -1\n', 'totals: 0\n',
                     'totals: 18446744073709551616\n', 'totals: nope\n',
                     'totals: 1\ntotals: 1\n', 'totals: ' + '1 ' * 14 + '\n']:
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.parse(self.header() + text)

    def test_invalid_events_fail_closed(self):
        for header in ['', self.header() * 2, 'events: Ir Ir\n', 'events: Ir\n']:
            with self.subTest(header=header), self.assertRaises(ValueError):
                self.parse(header + 'totals: 1\n')

if __name__ == '__main__':
    unittest.main()
