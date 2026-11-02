import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    'signed_mask_oracles', ROOT / 'scripts/check_signed_mask_oracles.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class SignedMaskOracleContracts(unittest.TestCase):
    def test_full_matrix_and_disable_control(self):
        rows = list(m.matrix('candidate', 'gcc-exact', 'clang-exact'))
        self.assertEqual(len(rows), 32)
        self.assertEqual(len(set(rows)), 32)
        for label in ('gcc', 'clang', 'lccc', 'lccc-no-ivwiden'):
            arm = [row for row in rows if row[0] == label]
            self.assertEqual({(row[3], row[4]) for row in arm},
                             {(abi, opt) for abi in ('-m64', '-m32')
                              for opt in ('-O1', '-O2', '-O3', '-Os')})
            self.assertTrue(all(row[2] == (label == 'lccc-no-ivwiden') for row in arm))
        self.assertEqual({row[1] for row in rows if row[0].startswith('lccc')}, {'candidate'})

    def test_success_requires_both_statuses_and_exact_checksum(self):
        good = {'status': 0, 'stdout': m.EXPECTED}
        self.assertTrue(m.accepted({'status': 0}, good))
        for status in (1, 97, -11, 'timeout', None, False):
            with self.subTest(status=status):
                self.assertFalse(m.accepted({'status': status}, good))
                self.assertFalse(m.accepted({'status': 0}, {**good, 'status': status}))
        for output in ('', m.EXPECTED.rstrip(), m.EXPECTED.replace('770224292', '0')):
            self.assertFalse(m.accepted({'status': 0}, {'status': 0, 'stdout': output}))
        self.assertFalse(m.accepted({}, good))
        self.assertFalse(m.accepted({'status': 0}, {}))


if __name__ == '__main__':
    unittest.main()
