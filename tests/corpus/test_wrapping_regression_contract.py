"""Wrapping witnesses must reach both optimized compiler arms with defined semantics."""
import importlib.util
from pathlib import Path
import shlex
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('wrapping_regression_runner', ROOT / 'tests/regression/run_regression.py')
runner = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = runner
spec.loader.exec_module(runner)


class WrappingContracts(unittest.TestCase):
    def test_real_discovery_and_both_compiler_commands(self):
        cases = {t.name: t for t in runner.discover([ROOT / 'tests/regression'])}
        for name in ('affine_control_proof', 'wrapv_domains', 'ivw_wrapv_range', 'ivw_signed_mask'):
            case = cases[name]
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                self.assertFalse(case.no_compare, 'keep the independent oracle enabled')
                self.assertEqual(shlex.split(case.flags), ['-O2', '-fwrapv'])
                work = Path(directory)
                (work / name).touch()
                outcomes = [(0, '', ''), (0, 'expected\n', ''),
                            (0, '', ''), (0, 'expected\n', '')]
                with patch.object(runner, 'run_checked', side_effect=outcomes) as run:
                    result = runner.compile_one(Path('lccc'), 'gcc', case, work)
                self.assertEqual(result.status, 'pass')
                self.assertEqual(run.call_count, 4)
                for index in (0, 2):
                    command = run.call_args_list[index].args[0]
                    self.assertIn('-O2', command)
                    self.assertIn('-fwrapv', command)

    def test_bad_oracle_exit_or_output_still_fails(self):
        case = next(t for t in runner.discover([ROOT / 'tests/regression'])
                    if t.name == 'affine_control_proof')
        for oracle in [(1, 'expected\n', ''), (0, 'wrong\n', '')]:
            with self.subTest(oracle=oracle), tempfile.TemporaryDirectory() as directory:
                work = Path(directory)
                (work / case.name).touch()
                with patch.object(runner, 'run_checked', side_effect=[
                        (0, '', ''), (0, 'expected\n', ''), (0, '', ''), oracle]):
                    result = runner.compile_one(Path('lccc'), 'gcc', case, work)
                self.assertEqual(result.status, 'fail')
                self.assertIn('compare:fail', result.phases)

    def test_independent_signed_ring_expected_values(self):
        def signed(x): return ((x + (1 << 31)) % (1 << 32)) - (1 << 31)
        step = 1073741823
        i = count = 0
        while count < 32:
            if i > 0 and i != step and signed(i + 2) >= 100:
                break
            count += 1
            i = signed(i + step)
        self.assertEqual(count, 5)
        i = count = total = 0
        while signed(i + 1073741830) < 2147483644:
            self.assertLess(count, 32, 'model unexpectedly cycles')
            total += signed(i + 100) < 1000
            count += 1
            i = signed(i + step)
        self.assertEqual((count, total, 10 * count + total), (9, 7, 97))


if __name__ == '__main__':
    unittest.main()
