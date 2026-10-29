#!/usr/bin/env python3
"""Offline adversarial contracts for the qualifier gate; never invokes a compiler."""
import subprocess
import unittest
from unittest.mock import patch

import check_qualifier_conversions as gate


class ResultContracts(unittest.TestCase):
    def result(self, rc, text):
        return gate.parse_result(subprocess.CompletedProcess([], rc, '', text), gate.SOURCE)

    def test_consistent_results(self):
        for severity, rc in [('warning', 0), ('error', 1)]:
            with self.subTest(severity=severity):
                result = self.result(rc, f'{gate.SOURCE.name}:1:2: {severity}: message [-Wtest]\n')
                self.assertEqual(result.returncode, rc)
                self.assertEqual(len(result.diagnostics), 1)
        self.assertEqual(self.result(0, '').diagnostics, [])

    def test_inconsistent_exits_and_crashes(self):
        for rc, text in [(0, 'error'), (1, 'warning'), (-11, 'error'), (101, 'error')]:
            with self.subTest(rc=rc, severity=text), self.assertRaises(SystemExit):
                self.result(rc, f'{gate.SOURCE.name}:1:2: {text}: message\n')
        with self.assertRaises(SystemExit):
            self.result(1, '')

    def test_unexpected_output_never_disappears(self):
        baseline = f'{gate.SOURCE.name}:1:2: error: message\n'
        for extra in ['lccc: fatal error: missing input', "thread 'main' panicked at src/test.rs:1",
                      'internal compiler error: broken', 'other.c:2:3: error: bad',
                      f'{gate.SOURCE.name}:2: error: missing column', 'unexpected output']:
            with self.subTest(extra=extra), self.assertRaises(SystemExit):
                self.result(1, baseline + extra + '\n')

    def test_rendering_and_notes(self):
        text = (f"{gate.SOURCE.name}: In function 'f':\n"
                f'{gate.SOURCE.name}:1:2: error: message\n'
                '    1 | int *x = 1;\n      |  ^~~~\n'
                f'{gate.SOURCE.name}:1:1: note: declared here\n')
        self.assertEqual(len(self.result(1, text).diagnostics), 1)

    def test_summary_count_must_match_primary_errors(self):
        baseline = f'{gate.SOURCE.name}:1:2: error: message\n'
        good = 'ccc: error: 1 error(s) during semantic analysis\n'
        self.assertEqual(self.result(1, baseline + good).returncode, 1)
        for summary in ['ccc: error: 2 error(s) during semantic analysis\n', good + good]:
            with self.subTest(summary=summary), self.assertRaises(SystemExit):
                self.result(1, baseline + summary)

    def test_success_with_expected_errors_is_not_a_passing_gate(self):
        rows = ''.join(f'{gate.SOURCE.name}:{line}:1: {sev}: message' +
                       ('' if opt == 'none' else f' [-W{opt}]') + '\n'
                       for line, (sev, opt) in gate.expectations().items())
        with self.assertRaises(SystemExit):
            self.result(0, rows)
        with self.assertRaises(SystemExit):
            self.result(1, rows + 'ccc: fatal error: driver failed\n')

    def test_oracle_missing_requires_explicit_opt_out(self):
        with patch.object(gate.shutil, 'which', return_value=None):
            with self.assertRaises(SystemExit):
                gate.oracle_path('missing', False)
            self.assertIsNone(gate.oracle_path('missing', True))

    def test_wrong_oracle_rejected(self):
        with patch.object(gate.shutil, 'which', return_value='/mock/gcc'), \
             patch.object(gate.subprocess, 'run', return_value=
                          subprocess.CompletedProcess([], 0, '16.1.0\n', '')):
            with self.assertRaises(SystemExit):
                gate.oracle_path('gcc', False)

    def test_duplicate_diagnostic_rejected(self):
        row = dict(line='1', sev='error', opt=None)
        with self.assertRaises(SystemExit):
            gate.classified([row, row])


if __name__ == '__main__':
    unittest.main()
