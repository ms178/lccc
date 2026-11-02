import unittest
from aos_pair_shapes import ProbeError
from check_lightpixel import body, NAMES
from benchmark_lightpixel import receipt


class LightPixelContracts(unittest.TestCase):
    def test_inventory_is_independent_and_complete(self):
        self.assertEqual(len(NAMES), 12)
        self.assertEqual(len(set(NAMES)), 12)
        self.assertIn('lightpixel_interference', NAMES)
        self.assertIn('lightpixel_volatile', NAMES)
        self.assertIn('lightpixel_separate', NAMES)

    def test_assembly_body_stops_at_own_size(self):
        text = 'umin4:\n vpminud %xmm0, %xmm1, %xmm2\n .size umin4, .-umin4\numax4:\n ret\n'
        self.assertIn('vpminud', body(text, 'umin4'))
        self.assertNotIn('ret', body(text, 'umin4'))

    def test_missing_or_incomplete_symbol_fails(self):
        for text in ['', 'umin4:\n ret\n', 'umin40:\n .size umin40, 2\n']:
            with self.subTest(text=text), self.assertRaises(ProbeError):
                body(text, 'umin4')

    def test_benchmark_receipt_checks_kernel_and_full_output(self):
        self.assertEqual(receipt('2 0123456789abcdef 123\n', 2), ('0123456789abcdef', 123))
        for text in ['', '2 0123456789abcdef 0\n', '2 0123456789abcdef -1\n',
                     '1 0123456789abcdef 123\n', '2 0123 123\n',
                     '2 0123456789abcdef nan\n', '2 0123456789abcdef 123\nextra\n']:
            with self.subTest(text=text), self.assertRaises(ProbeError):
                receipt(text, 2)


if __name__ == '__main__':
    unittest.main()
