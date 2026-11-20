"""Offline contracts: coverage and transport failures must not look like wins."""
import contextlib
import io
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
