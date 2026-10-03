"""LCCC's counted summaries: source-checked native adapter, no broad exemption."""
from pathlib import Path
import sys
import unittest

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO))
from tools.corpus import diagnostics

TEXT = 'x.c:19:6: error: too many arguments\nccc: error: 1 error(s) during semantic analysis\n'


class NativeSummaryTests(unittest.TestCase):
    def test_generic_or_other_driver_not_exempt(self):
        self.assertEqual(diagnostics.classify(1, TEXT), 'ERROR')
        self.assertEqual(diagnostics.classify(1, TEXT, compiler_family='gcc'), 'ERROR')

    def test_lccc_exact_counted_summary_is_not_extra_error(self):
        records = diagnostics.parse(TEXT, compiler_family='lccc', returncode=1)
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]['file'], 'x.c')
        self.assertEqual(diagnostics.classify(1, TEXT, compiler_family='lccc'), 'REJECT')

    def test_all_actual_pipeline_summary_forms(self):
        for summary in ('x.c: 1 parse error(s)', 'x.c: 1 frontend error(s)', '1 preprocessor error(s) in x.c',
                        '1 error(s) (warnings promoted by -Werror)'):
            text = 'x.c:1:1: error: bad\nccc: error: ' + summary
            self.assertEqual(diagnostics.classify(1, text, compiler_family='lccc'), 'REJECT')

    def test_wrong_count_unknown_summary_or_success_not_exempt(self):
        for text, code in ((TEXT.replace('1 error(s)', '2 error(s)'), 1),
                           (TEXT.replace('during semantic analysis', 'unknown driver failure'), 1),
                           (TEXT, 0), (TEXT, 127),
                           ('ccc: error: 1 error(s) during semantic analysis', 1),
                           (TEXT + 'ccc: error: unrelated transport failure', 1)):
            self.assertEqual(diagnostics.classify(code, text, compiler_family='lccc'), 'ERROR')

    def test_native_parity_does_not_weaken_actual_location_or_message(self):
        records = diagnostics.parse(TEXT, compiler_family='lccc', returncode=1)
        e = dict(id='e', kind='error', regex_form=False, message='too many arguments',
                 count_min=1, count_max=1, location=dict(file='$SOURCE', line=19, any_file=False))
        self.assertTrue(diagnostics.verify([e], records, source=Path('x.c'))['ok'])
        e['location']['line'] = 20
        self.assertFalse(diagnostics.verify([e], records, source=Path('x.c'))['ok'])


if __name__ == '__main__':
    unittest.main()
