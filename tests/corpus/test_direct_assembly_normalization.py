"""Only genuinely symmetric register operands can be normalized for parity."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('direct_parity', Path(__file__).resolve().parents[1] / 'regression/check_direct_assembly_parity.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

class IntegerSourceOrder(unittest.TestCase):
    def test_signed_word_pair_products_are_symmetric(self):
        self.assertEqual(m.canonical_commutative('vpmaddwd %ymm8,%ymm0,%ymm0'),
                         m.canonical_commutative('vpmaddwd %ymm0,%ymm8,%ymm0'))

    def test_unsigned_signed_byte_products_are_not_symmetric(self):
        for op in ('vpmaddubsw', 'vpsubd', 'vpalignr'):
            text = op + ' %ymm8,%ymm0,%ymm0'
            self.assertEqual(m.canonical_commutative(text), text)

    def test_memory_address_commas_are_not_register_operands(self):
        text = 'vpmaddwd (%rax,%rdx,4),%ymm8,%ymm0'
        self.assertEqual(m.canonical_commutative(text), text)
