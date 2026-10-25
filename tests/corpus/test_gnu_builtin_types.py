"""Pure contracts for native GNU builtin metadata and its two generated views."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('gnu_builtin_generator', ROOT / 'scripts/gen_x86_builtin_types.py')
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class NativeBuiltinTypes(unittest.TestCase):
    def test_generated_views_are_current(self):
        self.assertEqual(generator.OUTPUT.read_text(), generator.generate())
        self.assertEqual(generator.CONTRACT.read_text(), generator.contract_source())

    def test_hosted_dual_mode_fixtures_have_real_i686_entries(self):
        for family in ('gnu_native_builtin_types', 'gnu_byte_shift_counts', 'public_simd_native_types'):
            with self.subTest(family=family):
                directory = ROOT / 'tests/regression'
                entry = (directory / f'{family}_i686.c').read_text()
                self.assertIn(f'#include "{family}.c"', entry)
                flags = (directory / f'{family}_i686.flags').read_text().split()
                self.assertEqual(flags.count('-m32'), 1)
                self.assertNotIn('-m64', flags)
                flags.remove('-m32')
                self.assertEqual(flags, (directory / f'{family}.flags').read_text().split())

    def test_lane_width_is_not_ir_pointer_type(self):
        self.assertEqual(generator.result_expression('V4SI'), 'CType::Vector(Box::new(CType::Int), 16)')
        self.assertEqual(generator.result_expression('V4DI'), 'CType::Vector(Box::new(CType::LongLong), 32)')
        self.assertEqual(generator.result_expression('V8SF'), 'CType::Vector(Box::new(CType::Float), 32)')

    def test_unknown_result_and_argument_are_errors(self):
        with self.assertRaises(ValueError):
            generator.result_expression('UNRECOGNIZED')
        with self.assertRaises(ValueError):
            generator.c_type('UNRECOGNIZED')

    def test_duplicate_facts_are_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            facts = Path(directory) / 'facts.tsv'
            facts.write_text('__builtin_x\tINT_FTYPE_VOID\tboth\n' * 2)
            with patch.object(generator, 'FACTS', facts), self.assertRaises(ValueError):
                generator.generate()

    def test_unknown_mode_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            facts = Path(directory) / 'facts.tsv'
            facts.write_text('__builtin_x\tINT_FTYPE_VOID\tunknown\n')
            with patch.object(generator, 'FACTS', facts), self.assertRaises(ValueError):
                generator.generate()

    def test_x64_only_facts_are_explicit(self):
        actual = {line.split('\t')[0] for line in generator.FACTS.read_text().splitlines()
                  if line.endswith('\tx86_64')}
        self.assertEqual(actual, {'__builtin_ia32_' + name for name in
                                 ('bzhi_di', 'crc32di', 'cvtsd2si64', 'cvtss2si64', 'movnti64')})

    def test_vnni_contracts_do_not_require_vnni_execution(self):
        source = generator.contract_source()
        for line in source.splitlines():
            if '__builtin_ia32_vpdp' in line:
                self.assertTrue(line.startswith('_Static_assert('), line)


if __name__ == '__main__':
    unittest.main()
