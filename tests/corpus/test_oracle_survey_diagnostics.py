"""Failed/incomplete surveys must remain visible, bounded and unranked."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import oracle_survey_diagnostics as tool


class SurveyDiagnostics(unittest.TestCase):
    def run_report(self, index=None, report=None):
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / 'index.json'
            if index is not None:
                path.write_text(json.dumps(index))
            if report is not None:
                result = Path(name) / 'result.json'
                result.write_text(json.dumps(report))
                for row in index['results'].values():
                    row['report'] = str(result)
                path.write_text(json.dumps(index))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(tool.main([str(path)]), 0)
            lines = output.getvalue().splitlines()
            decoded = []
            for line in lines:
                self.assertLessEqual(len(line.split('::', 2)[2].encode()), tool.BUDGET)
                self.assertTrue(line.startswith('::notice title=Oracle survey diagnostics::'))
                text = line.split('::', 2)[2].replace('%0D', '\r').replace('%0A', '\n').replace('%25', '%')
                decoded.append(json.loads(text))
            return decoded

    def test_missing_or_malformed_index_is_not_a_pass(self):
        for index in (None, {}, {'results': [], 'failures': []}):
            with self.subTest(index=index):
                row = self.run_report(index)[0]
                self.assertFalse(row['available'])
                self.assertFalse(row['comparison_pass'])

    def test_failed_report_keeps_vendor_rejection_and_request(self):
        rows = self.run_report({'complete': True, 'failures': [{'name': 'nbody', 'exit_code': 1}],
                                'results': {'nbody': {'arguments': ['--oracles', 'gcc16.2,clang,icc,icx',
                                                                  '--flags', '-O2 -march=x86-64-v3']}}},
                               [{'errors': [{'compiler': 'icc', 'error': 'warning #10148: option not supported'}],
                                 'records': [{'key': 'lccc'}, {'key': 'gcc16.2'}]}])
        self.assertEqual(rows[0]['failed_requests'], 1)
        self.assertFalse(rows[0]['comparison_pass'])
        self.assertIn('10148', rows[1]['errors'][0])
        self.assertIn('icc', rows[1]['requested_oracles'])
        self.assertEqual(rows[1]['requested_flags'], '-O2 -march=x86-64-v3')
        self.assertNotIn('rank', str(rows))

    def test_empty_successful_index_is_diagnostics_not_acceptance(self):
        row = self.run_report({'complete': True, 'results': {}, 'failures': []})[0]
        self.assertFalse(row['comparison_pass'])
        self.assertEqual(row['recorded_requests'], 0)

    def test_large_failure_set_and_escaped_diagnostics_fit_wire_budget(self):
        names = [f'case{i}' for i in range(45)]
        rows = self.run_report({'complete': False, 'failures': [{'name': n} for n in names],
                                'results': {n: {'arguments': ['--oracles', '%' * 1000,
                                                             '--flags', '汉%' * 1000]} for n in names}},
                               [{'errors': [{'error': '%' * 10000} for _ in range(4)], 'records': []}])
        self.assertEqual(len(rows), tool.MAX_DETAILS + 2)
        self.assertEqual(rows[-1]['omitted_failure_details'], 45 - tool.MAX_DETAILS)
        self.assertTrue(any(r.get('detail_truncated') for r in rows))


if __name__ == '__main__':
    unittest.main()
