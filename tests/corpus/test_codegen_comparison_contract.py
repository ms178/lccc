"""Offline contracts: coverage and transport failures must not look like wins."""
import contextlib
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import godbolt_codegen_compare as cmp


class ComparisonContracts(unittest.TestCase):
    def test_missing_input_reports_failure_without_empty_row_crash(self):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(cmp.main(['--local', 'none', '--offline', 'no-such-source.c']), 1)

    def test_offline_cache_miss_never_opens_transport(self):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / 'input.c'
            src.write_text('int f(void) { return 1; }')
            with patch.object(cmp.godbolt, 'CACHE', Path(tmp)), \
                 patch.object(cmp.co.godbolt_cache, 'load_lines', return_value=None), \
                 patch('urllib.request.urlopen') as transport, \
                 contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(cmp.main([str(src), '--offline', '--local', 'none', '--oracles', 'gcc']), 1)
                transport.assert_not_called()

    def run_mock_survey(self, listings, extra=()):
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / 'input.c'
            report = Path(tmp) / 'report.json'
            src.write_text('int f(void) { return 1; }')
            with patch.object(cmp, '_remote_lines', side_effect=listings), \
                 contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                result = cmp.main([str(src), '--local', 'none', '--oracles', 'gcc,clang',
                                   '--json', str(report), *extra])
            return result, json.loads(report.read_text())

    def test_partial_compiler_failure_is_not_a_successful_survey(self):
        status, report = self.run_mock_survey([
            (['f:', '  movl $1, %eax', '  ret'], ''),
            ([], 'transport unavailable'),
        ])
        self.assertEqual(status, 1)
        self.assertEqual(len(report['failures']), 1)
        self.assertEqual(report['rows'][0]['compiler_errors'], {'clang': 'transport unavailable'})

    def test_oracles_only_function_does_not_invent_a_missing_lccc(self):
        status, report = self.run_mock_survey([
            (['f:', '  movl $1, %eax', '  ret'], ''),
            (['f:', '  movl $1, %eax', '  ret'], ''),
        ], ['--function', 'f'])
        self.assertEqual(status, 0)
        self.assertEqual(report['failures'], [])
        self.assertNotIn('lccc', report['rows'][0]['errors'])

    def test_inlining_is_distinct_from_a_failed_compiler(self):
        status, report = self.run_mock_survey([
            (['f:', '  ret', 'g:', '  ret'], ''),
            (['f:', '  ret'], ''),
        ])
        self.assertEqual(status, 0)
        self.assertTrue(any(row['errors'] for row in report['rows']))
        self.assertTrue(all(not row['compiler_errors'] for row in report['rows']))

    def test_wins_and_gaps_keep_opposite_signs_and_report_extremes(self):
        def row(name, mine, ref):
            return cmp.Row('test.c', name, {
                'lccc': cmp.co.AsmStats(instructions=mine),
                'gcc': cmp.co.AsmStats(instructions=ref),
            }, {})
        summary = cmp.summarize([row('regression', 20, 10), row('gain', 5, 10)], ['gcc'], 'static')
        self.assertEqual(summary['geomean_lccc_over_best'], 1.0)
        self.assertEqual(summary['arithmetic_mean_lccc_over_best'], 1.25)
        self.assertEqual(summary['best_lccc_over_best'], 0.5)
        self.assertEqual(summary['worst_lccc_over_best'], 2.0)
        self.assertEqual([r['function'] for r in summary['top_wins']], ['gain'])
        self.assertEqual([r['function'] for r in summary['top_gaps']], ['regression'])

    def test_outlined_helper_cannot_win_by_shrinking_only_the_caller(self):
        row = cmp.Row('crc.c', 'combine', {
            'lccc': cmp.co.AsmStats(instructions=2, calls=1),
            'gcc': cmp.co.AsmStats(instructions=50, calls=0),
        }, {})
        self.assertFalse(row.comparable_call_scope)
        self.assertIsNone(row.lccc_over_best)
        self.assertEqual(row.best_name, '')
        self.assertEqual(row.to_dict()['stats']['lccc']['instructions'], 2)
        self.assertIn('helpers', row.to_dict()['scope_warning'])
        summary = cmp.summarize([row], ['gcc'], 'static')
        self.assertEqual(summary['ranked'], 0)
        self.assertEqual(summary['top_wins'], [])
        with contextlib.redirect_stdout(io.StringIO()) as output:
            cmp.render([row], ['gcc'], 'static')
        self.assertIn('not ranked: call scopes differ', output.getvalue())

    def test_strict_gate_refuses_incomparable_call_scopes(self):
        status, report = self.run_mock_survey([
            (['f:', '  call helper', '  ret'], ''),
            (['f:', '  movl $1, %eax', '  ret'], ''),
        ], ['--fail-on-behind'])
        self.assertEqual(status, 1)
        self.assertTrue(any('incomparable static call scopes' in s for s in report['failures']))

    def test_native_oracle_is_dispatched_locally_and_retains_identity(self):
        metadata = {'id': 'icx2026.2', 'semver': '2026.2.1', 'binary_sha256': 'a' * 64}
        with patch.object(cmp.godbolt, 'oracle_assembly', return_value=(metadata, ['f:', '  ret'])) as native, \
             patch.object(cmp, '_remote_lines') as remote:
            row, = cmp.compare_file(Path(__file__), local='', oracles=['icx2026.2'],
                                    plan=cmp.FlagPlan('-O2'), function='f', artifact_dir=None,
                                    mode='static')
        native.assert_called_once_with('icx2026.2', Path(__file__), '-O2')
        remote.assert_not_called()
        self.assertEqual(row.to_dict()['native_oracles'], {'icx2026.2': metadata})
        self.assertFalse(row.errors)

    def test_offline_native_resolution_never_starts_a_provisioner(self):
        token = cmp.godbolt.CACHE_ONLY.set(True)
        try:
            with patch.dict(os.environ, {}, clear=True), \
                 patch.dict(cmp.godbolt._LOCAL_BINARIES, {}, clear=True), \
                 patch.object(cmp.godbolt.subprocess, 'run') as process:
                with self.assertRaisesRegex(cmp.godbolt.GodboltError, 'offline'):
                    cmp.godbolt.local_oracle_binary('icx2026.2')
                process.assert_not_called()
        finally:
            cmp.godbolt.CACHE_ONLY.reset(token)

    def test_remote_runtime_mode_refuses_native_aliases_before_transport(self):
        with patch.object(cmp.godbolt, 'compile_on_godbolt') as remote, \
             contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as status:
                cmp.main([str(Path(__file__)), '--local', 'none', '--mode', 'runtime',
                          '--oracles', 'icx2026.2'])
            self.assertEqual(status.exception.code, 2)
            remote.assert_not_called()

    def test_executor_status_not_build_status_and_one_row_per_program(self):
        results = [{'execResult': {'code': 2, 'stdout': [{'text': '10 ms'}]}},
                   {'execResult': {'code': 0, 'stdout': [{'text': '3 ms'}]}}]
        with patch.object(cmp.godbolt, 'compile_on_godbolt', side_effect=results):
            row, = cmp.compare_file(Path(__file__), local='', oracles=['gcc', 'clang'],
                                    plan=cmp.FlagPlan(''), function=None, artifact_dir=None,
                                    mode='runtime')
        self.assertEqual(row.runtime, {'clang': 0.003})
        self.assertIn('code 2', row.errors['gcc'])
        self.assertEqual(row.best_name, 'clang')
        self.assertIsNone(row.lccc_runtime_over_best)


if __name__ == '__main__':
    unittest.main()
