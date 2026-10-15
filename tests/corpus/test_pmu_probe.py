"""Perf exit zero is not proof of usable hardware events."""
import os
from pathlib import Path
import sys
import unittest
from unittest import mock
ROOT = Path(os.environ.get('LCCC_TEST_REPO', Path(__file__).resolve().parents[2]))
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as runner

class PmuProbe(unittest.TestCase):
    def probe(self, stderr, ok=True):
        result = {'ok': ok, 'returncode': 0 if ok else 1, 'stderr': stderr, 'stdout': '', 'error': ''}
        with mock.patch.object(runner.shutil, 'which', return_value='/perf'), \
             mock.patch.object(runner, 'read_first', return_value='2'), \
             mock.patch.object(runner, 'invoke', return_value=result):
            return runner.probe_pmu(True)

    def test_usable_user_counters(self):
        result = self.probe('100,,cycles:u,1,100.00,,\n200,,instructions:u,1,100.00,,\n')
        self.assertTrue(result['available'])
        self.assertEqual(result['observed_counts'], {'cycles': 100, 'instructions': 200})
        self.assertIn('-x', result['probe_command'])

    def test_zero_exit_unsupported_is_not_available(self):
        self.assertFalse(self.probe('<not supported>,,cycles:u,0,100.00,,\n<not supported>,,instructions:u,0,100.00,,')['available'])

    def test_missing_empty_and_not_counted_are_not_available(self):
        for text in ['', '100,,cycles,1,100,,', '100,,cycles,1,100,,\n<not counted>,,instructions,0,0,,']:
            self.assertFalse(self.probe(text)['available'])

    def test_zero_negative_and_nonfinite_counts_are_not_available(self):
        for value in ['0', '-1', 'nan', 'inf']:
            self.assertFalse(self.probe(f'{value},,cycles,1,100,,\n200,,instructions,1,100,,')['available'])

    def test_failed_command_and_duplicate_events_are_not_available(self):
        text = '100,,cycles,1,100,,\n200,,instructions,1,100,,\n'
        self.assertFalse(self.probe(text, False)['available'])
        self.assertFalse(self.probe(text + '110,,cycles,1,100,,\n')['available'])

    def test_disabled_probe_never_executes_perf(self):
        with mock.patch.object(runner, 'invoke') as invoke:
            self.assertFalse(runner.probe_pmu(False)['available'])
            invoke.assert_not_called()

if __name__ == '__main__':
    unittest.main()
