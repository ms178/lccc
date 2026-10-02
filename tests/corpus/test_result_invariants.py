"""Adversarial control-field and process-evidence mutations, not native passes."""
import copy
import importlib.util
from pathlib import Path
import sys
import unittest

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))
from tools.corpus import process, schema
spec = importlib.util.spec_from_file_location('result_runner', REPO / 'tests/corpus/run_clang_c_corpus.py')
runner = importlib.util.module_from_spec(spec); spec.loader.exec_module(runner)


class ResultInvariantTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        doc = schema.loads((REPO / 'tests/corpus/clang-c/corpus-index.json').read_text())
        rec = next(r for r in doc['files'] if r['file'])
        cls.row = runner._row(rec, rec['invocations'][0])

    def test_full_unexecuted_row_valid(self):
        schema.validate_results([copy.deepcopy(self.row)])

    def test_unknown_missing_fields_not_accepted(self):
        r = copy.deepcopy(self.row); r['fake_field'] = True
        with self.assertRaises(ValueError): schema.validate_results([r])
        del r['fake_field']; del r['attempted']
        with self.assertRaises(ValueError): schema.validate_results([r])

    def test_false_boolean_and_integer_subtypes(self):
        for field, value in [('planned', 1), ('attempted', 0), ('xfail', 'false'),
                             ('diagnostics_verified', 1), ('returncode', True),
                             ('elapsed', True), ('diagnostic_count', False), ('id', [])]:
            r = copy.deepcopy(self.row); r[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                schema.validate_results([r])

    def test_unexecuted_pass_or_certificate_never_valid(self):
        for mutation in ({'outcome': 'ACCEPT', 'status': 'PASS'},
                         {'diagnostics_verified': True},
                         {'spawned': True, 'executed': True},
                         {'translation_key': 'arbitrary'}):
            r = copy.deepcopy(self.row); r.update(mutation)
            with self.assertRaises(ValueError): schema.validate_results([r])

    def test_raw_process_hashes_and_shape_validated(self):
        valid = process.normalize((0, b'out', b'err'))
        for field, value in [('returncode', True), ('stdout_sha256', '0' * 64),
                             ('stdout', 'not bytes'), ('elapsed', float('inf'))]:
            bad = dict(valid); bad[field] = value
            with self.subTest(field=field), self.assertRaises(ValueError): process.normalize(bad)
        with self.assertRaises(ValueError): process.normalize({'returncode': 0})


if __name__ == '__main__': unittest.main()
