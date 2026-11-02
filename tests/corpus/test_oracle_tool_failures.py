"""Offline red-team tests: missing or stale oracle data must not look successful."""
import argparse
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import godbolt
import oracle_asm
import tune_oracle

ASM = ['foo:', '    ret']

class OracleFailures(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'probe.c'
        self.source.write_text('void foo(void) {}\n')

    def dump(self, local=ASM, remote=ASM, function='foo'):
        args = [str(self.source), '--flags', '-O2', '--function', function,
                '--oracles', 'gcc16.2', '--out', str(self.root)]
        def mocked(value):
            return {'side_effect': value} if isinstance(value, Exception) else {'return_value': value}
        with patch.object(oracle_asm.oracle, '_local_compile', **mocked(local)), \
             patch.object(oracle_asm.oracle, '_compile_remote', **mocked(remote)), \
             contextlib.redirect_stdout(io.StringIO()):
            return oracle_asm.main(args)

    def test_dump_success(self):
        self.assertEqual(self.dump(), 0)
        self.assertTrue((self.root / 'gcc16.2.s').exists())

    def test_local_failure_is_nonzero_and_removes_stale_local_output(self):
        (self.root / 'lccc.s').write_text('stale')
        self.assertEqual(self.dump(local=RuntimeError('local failed')), 1)
        self.assertFalse((self.root / 'lccc.s').exists())

    def test_remote_failure_is_nonzero_and_removes_stale_remote_output(self):
        (self.root / 'gcc16.2.s').write_text('stale')
        self.assertEqual(self.dump(remote=RuntimeError('remote failed')), 1)
        self.assertFalse((self.root / 'gcc16.2.s').exists())

    def test_missing_function_is_not_success(self):
        self.assertEqual(self.dump(function='absent'), 1)

    def test_tuning_labels_allow_ce_comments_and_quoted_symbols(self):
        self.assertEqual(set(tune_oracle.split_functions('"foo": # @foo\n ret\n')), {'foo'})

    def test_tuning_missing_and_empty_responses_fail(self):
        for response in [None, {'asm': []}]:
            with self.subTest(response=response), patch.object(godbolt, 'compile_on_godbolt', return_value=response):
                with self.assertRaises(godbolt.GodboltError):
                    tune_oracle.godbolt_compile('gcc16.2', self.source, '-O2')

    def test_tuning_compare_refuses_partial_table_and_stale_output(self):
        stale = self.root / 'probe.gcc16.2.s'; stale.write_text('stale')
        args = argparse.Namespace(probe=str(self.source), lccc='unused', flags='-O2',
                                  oracles='gcc16.2', function='foo', save=str(self.root))
        with patch.object(tune_oracle, 'lccc_compile', return_value='foo:\n ret'), \
             patch.object(tune_oracle, 'godbolt_compile', side_effect=RuntimeError('unavailable')), \
             contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(tune_oracle.cmd_compare(args), 1)
        self.assertFalse(stale.exists())

    def test_local_abort_cannot_leave_a_success_status(self):
        report = self.root / 'status.json'
        report.write_text('{"complete": true}')
        args = argparse.Namespace(probe=str(self.source), lccc='unused', flags='-O2',
                                  oracles='gcc16.2', function='foo', save=str(self.root))
        with patch.object(tune_oracle, 'lccc_compile', side_effect=RuntimeError('failed')):
            with self.assertRaises(RuntimeError):
                tune_oracle.cmd_compare(args)
        self.assertFalse(json.loads(report.read_text())['complete'])

    def test_execute_cli_rejects_program_failure_and_missing_execution(self):
        args = argparse.Namespace(compiler='gcc16.2', source=self.source, flags='-O2',
                                  intel=False, execute=True, function=None)
        for execution, expected in [({'code': 0, 'didExecute': True}, 0),
                                    ({'code': 23}, 1), ({}, 1),
                                    ({'code': 0, 'timedOut': True}, 1),
                                    ({'code': 0, 'didExecute': False}, 1)]:
            result = {'asm': [{'text': line} for line in ASM], 'execResult': execution}
            with self.subTest(execution=execution), \
                 patch.object(godbolt, 'compile_on_godbolt', return_value=result), \
                 contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(godbolt.command_compile(args), expected)

    def test_shared_local_helpers_reject_missing_and_empty_output(self):
        for helper in [godbolt._compile_local, oracle_asm.oracle._local_compile]:
            for empty in [False, True]:
                def compile_(command, **kwargs):
                    if empty:
                        Path(command[-1]).write_text('')
                    return subprocess.CompletedProcess(command, 0, '', '')
                with self.subTest(helper=helper, empty=empty), \
                     patch.object(godbolt.subprocess, 'run', side_effect=compile_):
                    with self.assertRaises(godbolt.GodboltError):
                        helper('unused', self.source, '-O2')

    def test_shared_local_helper_cleans_failed_compile_output(self):
        outputs = []
        def compile_(command, **kwargs):
            out = Path(command[-1]); outputs.append(out)
            out.write_text('partial assembly')
            return subprocess.CompletedProcess(command, 1, '', 'synthetic error')
        with patch.object(godbolt.subprocess, 'run', side_effect=compile_):
            with self.assertRaisesRegex(godbolt.GodboltError, 'synthetic error'):
                godbolt._compile_local('unused', self.source, '-O2')
        self.assertFalse(outputs[0].parent.exists())

    def test_shared_local_helper_reports_timeout_and_cleans_directory(self):
        outputs = []
        def compile_(command, **kwargs):
            outputs.append(Path(command[-1]))
            raise subprocess.TimeoutExpired(command, kwargs['timeout'])
        with patch.object(godbolt.subprocess, 'run', side_effect=compile_):
            with self.assertRaises(godbolt.GodboltError):
                godbolt._compile_local('unused', self.source, '-O2')
        self.assertFalse(outputs[0].parent.exists())

    def test_local_compile_uses_private_temp_and_preserves_quoted_flags(self):
        outputs = []
        def compile_(command, **kwargs):
            out = Path(command[command.index('-o') + 1]); outputs.append(out)
            self.assertIn('-DMSG=two words', command)
            out.write_text('foo:\n ret\n')
            return subprocess.CompletedProcess(command, 0, '', '')
        with patch.object(tune_oracle.subprocess, 'run', side_effect=compile_):
            for _ in range(2):
                self.assertIn('ret', tune_oracle.lccc_compile('unused', self.source, '-O2 -DMSG="two words"'))
        self.assertNotEqual(outputs[0], outputs[1])
        self.assertTrue(all(not p.exists() for p in outputs))

if __name__ == '__main__':
    unittest.main()
