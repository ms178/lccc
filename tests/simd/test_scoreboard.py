"""Adversarial tests for the checksum inventory/parser and reference gating."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import check_scoreboard as gate
from aos_pair_shapes import ProbeError


def output():
    return ''.join(f'{name} 0123456789abcdef 0.0000\n' for name in gate.NAMES)


class ScoreboardTests(unittest.TestCase):
    def test_complete_inventory(self):
        self.assertEqual(len(gate.NAMES), 44)
        self.assertEqual(len(set(gate.NAMES)), 44)
        self.assertEqual(len(gate.checksums(output())), 44)

    def test_rejects_incomplete_duplicate_and_unknown_rows(self):
        for bad in ('', '\n'.join(output().splitlines()[:-1]), output()+output().splitlines()[0],
                    output().replace('add_f32 ', 'unknown '), output()+'garbage\n'):
            with self.subTest(output=bad[:30]), self.assertRaises(ProbeError):
                gate.checksums(bad)

    def test_rejects_invalid_checksums_and_timings(self):
        for bad in ('xyz', '0123456789abcde', '0123456789abcdef0'):
            with self.subTest(checksum=bad), self.assertRaises(ProbeError):
                gate.checksums(output().replace('0123456789abcdef', bad, 1))
        for bad in ('nan', 'inf', '-1', 'junk'):
            with self.subTest(timing=bad), self.assertRaises(ProbeError):
                gate.checksums(output().replace('0.0000', bad, 1))

    def test_reference_failure_prevents_candidate_execution(self):
        with patch.object(gate, 'run', side_effect=ProbeError('reference compilation failed')) as run:
            with patch('builtins.print'):
                self.assertEqual(gate.main([]), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn('-O0', run.call_args.args[0])

    def test_mismatches_fail_with_complete_receipts(self):
        import json
        calls = iter(['', output(), '', output().replace('0123456789abcdef', '1123456789abcdef', 1),
                      '', output(), '', output()])
        with tempfile.TemporaryDirectory() as d, patch.object(gate, 'run', side_effect=lambda *a: next(calls)):
            dest = Path(d)/'receipt.json'
            with patch('builtins.print'):
                self.assertEqual(gate.main(['--json', str(dest)]), 1)
            rows = json.loads(dest.read_text())['rows']
            self.assertEqual(len(rows), 132)
            self.assertEqual(sum(not row['ok'] for row in rows), 1)

    def test_compilation_and_execution_use_checked_process_helper(self):
        with patch.object(gate, 'run', side_effect=['', output()]*4) as run, patch('builtins.print'):
            self.assertEqual(gate.main([]), 0)
            self.assertEqual(run.call_count, 8)
            for call in run.call_args_list[::2]:
                command = call.args[0]
                self.assertIn('-DREPS=1', command)
                self.assertIn('-ffp-contract=off', command)
                self.assertIn('-Werror', command)

    def test_injected_clock_failure_requires_exact_runtime_failure(self):
        good = ProbeError('probe: rc=70\nclock_gettime: injected failure')
        for bad in ('compile failed', 'probe: rc=-11', ''):
            effects = ['', output()]*4 + ['', ProbeError(bad) if bad else '', '', good]
            with self.subTest(failure=bad), patch.object(gate, 'run', side_effect=effects):
                with patch('builtins.print'):
                    self.assertEqual(gate.main(['--check-clock-failure']), 1)
        with patch.object(gate, 'run', side_effect=['', output()]*4 + ['', good, '', good]):
            with patch('builtins.print'):
                self.assertEqual(gate.main(['--check-clock-failure']), 0)


if __name__ == '__main__':
    unittest.main()
