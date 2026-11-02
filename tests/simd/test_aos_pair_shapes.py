"""Runner regression tests: no real compiler needed."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('aos', Path(__file__).with_name('aos_pair_shapes.py'))
aos = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(aos)


class RunnerTests(unittest.TestCase):
    def test_compile_failure_does_not_run_stale_executable(self):
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            exe = root / 'candidate'
            exe.write_text('stale')
            with patch.object(aos, 'run', side_effect=aos.ProbeError('compile failed')) as run:
                with self.assertRaises(aos.ProbeError):
                    aos.evaluate(['cc'], ['-O2'], root / 'test.c', exe, 1, 1)
                self.assertEqual(run.call_count, 1)
                self.assertFalse(exe.exists())

    def test_nonzero_and_signal_are_errors(self):
        for rc in (1, -11):
            with self.subTest(rc=rc), patch.object(aos.subprocess, 'Popen') as popen:
                popen.return_value.returncode = rc
                popen.return_value.communicate.return_value = ('', 'bad')
                with self.assertRaisesRegex(aos.ProbeError, f'rc={rc}'):
                    aos.run(['cc'], Path('.'), 1)
                self.assertTrue(popen.call_args.kwargs['start_new_session'])

    def test_timeout_and_missing_command_are_errors(self):
        with patch.object(aos.subprocess, 'Popen', side_effect=FileNotFoundError('cc')):
            with self.assertRaises(aos.ProbeError):
                aos.run(['cc'], Path('.'), 1)
        with patch.object(aos.subprocess, 'Popen') as popen, patch.object(aos.os, 'killpg') as kill:
            popen.return_value.pid = 12345
            popen.return_value.communicate.side_effect = [
                subprocess.TimeoutExpired(['cc'], 1), ('', '')]
            with self.assertRaises(aos.ProbeError):
                aos.run(['cc'], Path('.'), 1)
            kill.assert_called_once_with(12345, aos.signal.SIGKILL)

    def test_real_timeout_terminates_pipe_inheriting_descendant(self):
        import sys
        import time
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            code = ('import subprocess,sys,time; '
                    'p=subprocess.Popen([sys.executable,"-c","import time; time.sleep(60)"]); '
                    'open("child.pid","w").write(str(p.pid)); time.sleep(60)')
            start = time.monotonic()
            with self.assertRaises(aos.ProbeError):
                aos.run([sys.executable, '-c', code], root, 2)
            self.assertLess(time.monotonic()-start, 10)
            pid = int((root/'child.pid').read_text())
            # A killed orphan may briefly be a zombie awaiting init's reap.
            stat = Path(f'/proc/{pid}/stat')
            try:
                state = stat.read_text().rsplit(')', 1)[1].split()[0]
            except FileNotFoundError:
                pass
            else:
                self.assertEqual(state, 'Z')

    def test_empty_successful_output_is_not_an_oracle(self):
        with tempfile.TemporaryDirectory() as work, patch.object(aos, 'run', return_value=''):
            with self.assertRaisesRegex(aos.ProbeError, 'expected 8 output elements'):
                aos.evaluate(['cc'], [], Path(work)/'t.c', Path(work)/'exe', 1, 8)

    def test_command_is_split_without_shell_and_relative_path_is_absolute(self):
        self.assertEqual(aos.compiler('cc "-DVALUE=a b"'), ['cc', '-DVALUE=a b'])
        self.assertEqual(aos.compiler('./target/fastbuild/lccc')[0],
                         str(Path('target/fastbuild/lccc').resolve()))
        with self.assertRaises(ValueError):
            aos.compiler(' ')

    def test_failed_reference_skips_candidate_and_fails_suite(self):
        with patch.object(aos, 'evaluate', side_effect=aos.ProbeError('reference failed')) as evaluate:
            with patch('builtins.print'):
                self.assertEqual(aos.main([]), 1)
            self.assertEqual(evaluate.call_count, len(aos.BODIES))
            self.assertTrue(all(call.args[1] == ['-O0'] for call in evaluate.call_args_list))

    def test_mismatch_fails_suite(self):
        def evaluate(cc, flags, *args):
            return 'reference' if flags == ['-O0'] else 'wrong'
        with patch.object(aos, 'evaluate', side_effect=evaluate), patch('builtins.print'):
            self.assertEqual(aos.main([]), 1)

    def test_success_writes_complete_json(self):
        import json
        with tempfile.TemporaryDirectory() as work:
            path = Path(work)/'results.json'
            with patch.object(aos, 'evaluate', return_value='same'), patch('builtins.print'):
                self.assertEqual(aos.main(['--json', str(path)]), 0)
            rows = json.loads(path.read_text())['rows']
            self.assertEqual(len(rows), 21)
            self.assertTrue(all(row['ok'] for row in rows))

    def test_guarded_lengths_and_type_substitution(self):
        for n in (0, 1, 64, 65):
            for stride in (2, 3, 4):
                for ctype in ('float', 'double', 'int32_t', 'int16_t'):
                    text, count = aos.source('a_swapadd', aos.BODIES['a_swapadd'], n,
                                             'guard-end', ctype, stride, True)
                    self.assertEqual(count, max(1, stride*n))
                    self.assertNotIn('TRIP', text)
                    self.assertNotIn('BODY', text)
                    self.assertNotIn('AT_END', text)
                    self.assertIn(f'{ctype} *x', text)
                    self.assertIn(f'o[{stride}*i]', text)
                    self.assertIn('PROT_NONE', text)
        _, count = aos.source('e_stride3', aos.BODIES['e_stride3'], 64, 'guard-start', stride=4)
        self.assertEqual(count, 192)

    def test_invalid_timeouts_fail_before_running_compiler(self):
        for timeout in ('0', '-1', 'nan', 'inf'):
            with self.subTest(timeout=timeout), patch.object(aos, 'evaluate') as evaluate:
                with patch('sys.stderr'), self.assertRaises(SystemExit) as error:
                    aos.main(['--timeout', timeout])
                self.assertEqual(error.exception.code, 2)
                evaluate.assert_not_called()


if __name__ == '__main__':
    unittest.main()
