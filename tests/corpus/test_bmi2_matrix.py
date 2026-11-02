import unittest
from scripts import gen_bmi2_matrix as generator


class BMI2Matrix(unittest.TestCase):
    def test_generated_register_and_address_axes_are_current(self):
        for bits, directory in ((32, 'i686'), (64, '')):
            with self.subTest(bits=bits):
                path = generator.ROOT / 'tests/asm-diff' / directory / 'bmi2-matrix.casefile'
                self.assertEqual(path.read_text(), generator.matrix(bits))

    def test_register_triples_are_complete(self):
        for bits, registers, widths in ((32, 8, 1), (64, 16, 2)):
            first = generator.matrix(bits).split(';;; pdep-pext-address-fields')[0]
            self.assertEqual(sum(line.startswith(('pdep', 'pext')) for line in first.splitlines()),
                             2 * widths * registers ** 3)
