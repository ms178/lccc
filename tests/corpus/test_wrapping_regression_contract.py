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
        for name in ('affine_control_proof', 'wrapv_domains', 'ivw_wrapv_range', 'ivw_signed_mask', 'signed_scale_ring_wrap'):
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

    def test_sidecar_without_level_gains_o2_and_explicit_level_keeps_control(self):
        cases = {t.name: t for t in runner.discover([ROOT / 'tests/regression'])}
        # No -O in the sidecar: the -O2 default is prepended, the sidecar
        # content survives. Replacement semantics silently ran the whole
        # simd/vectorize family at -O0 in this runner only.
        for name, kept in (('simd_avx2_256', '-mavx2'),
                           ('vectorize_dot_product', '-ffast-math'),
                           ('wrapv_byte_address', '-fwrapv')):
            with self.subTest(name=name):
                flags = shlex.split(cases[name].flags)
                self.assertEqual(flags[0], '-O2', flags)
                self.assertIn(kept, flags)
        # An explicit level is authoritative: no second -O is injected.
        explicit = shlex.split(cases['segment_fill_copy_alias'].flags)
        self.assertIn('-O0', explicit)
        self.assertEqual([f for f in explicit if f.startswith('-O')], ['-O0'])

    def test_bad_oracle_exit_or_output_still_fails(self):
        # `affine_control_proof` ships no `.env` sidecar, so it declares no
        # oracle self-check waiver. Both a reference binary that fails the
        # fixture's own assertions and one that merely disagrees must stay a
        # hard failure; a blanket "nonzero oracle means skip" would turn the
        # comparison off for every row a stale oracle touches. The per-fixture
        # waiver path is pinned separately by
        # test_oracle_gap_is_only_waived_when_the_fixture_declares_it.
        case = next(t for t in runner.discover([ROOT / 'tests/regression'])
                    if t.name == 'affine_control_proof')
        self.assertFalse(case.oracle_selfcheck_skip,
                         "this case pins the undeclared-waiver path; pick a "
                         "fixture without an LCCC_ORACLE_SELFCHECK sidecar")
        for oracle in [(1, 'expected\n', ''), (0, 'wrong\n', '')]:
            with self.subTest(oracle=oracle), tempfile.TemporaryDirectory() as directory:
                work = Path(directory)
                (work / case.name).touch()
                with patch.object(runner, 'run_checked', side_effect=[
                        (0, '', ''), (0, 'expected\n', ''), (0, '', ''), oracle]):
                    result = runner.compile_one(Path('lccc'), 'gcc', case, work)
                self.assertEqual(result.status, 'fail')
                self.assertIn('compare:fail', result.phases)
                self.assertTrue(result.detail, "a failure must carry a reason")
        # A failed oracle self-check must name the escape hatch, so the
        # operator sees the divergence and the documented waiver together
        # rather than a bare red row.
        with tempfile.TemporaryDirectory() as directory:
            work = Path(directory)
            (work / case.name).touch()
            with patch.object(runner, 'run_checked', side_effect=[
                    (0, '', ''), (0, 'expected\n', ''), (0, '', ''),
                    (1, 'expected\n', '')]):
                result = runner.compile_one(Path('lccc'), 'gcc', case, work)
            self.assertIn('LCCC_ORACLE_SELFCHECK=skip', result.detail)

    def test_oracle_gap_is_only_waived_when_the_fixture_declares_it(self):
        # An oracle whose *binary* fails the fixture's assertions is a
        # reference divergence and must stay a hard failure, unless the
        # fixture states the toolchain gap in its own sidecar.  The rule is
        # per test on purpose: a blanket "nonzero oracle means skip" would
        # silently disable the comparison for every row a stale oracle
        # touches, which is exactly the failure mode this test pins.
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            (corpus / 'declared.c').write_text('int main(void) { return 0; }\n')
            (corpus / 'declared.env').write_text(
                '# reference toolchain predates the pinned semantics\n'
                'LCCC_ORACLE_SELFCHECK=skip\n')
            (corpus / 'undeclared.c').write_text('int main(void) { return 0; }\n')
            cases = {case.name: case for case in runner.discover([corpus])}
            self.assertTrue(cases['declared'].oracle_selfcheck_skip)
            self.assertFalse(cases['undeclared'].oracle_selfcheck_skip)
            for verdict, name in (('skip-compare', 'declared'), ('fail', 'undeclared')):
                with self.subTest(name=name), tempfile.TemporaryDirectory() as work_dir:
                    work = Path(work_dir)
                    (work / name).touch()
                    with patch.object(runner, 'run_checked', side_effect=[
                            (0, '', ''), (0, 'expected\n', ''), (0, '', ''), (1, '', '')]):
                        result = runner.compile_one(Path('lccc'), 'gcc', cases[name], work)
                    self.assertEqual(result.status, verdict, result.detail)
                    self.assertIn('compare:oracle-skip' if verdict == 'skip-compare'
                                  else 'compare:fail', result.phases)

    def test_oracle_selfcheck_sidecar_rejects_unknown_values(self):
        # Fail closed on a typo: silently treating `LCCC_ORACLE_SELFCHECK=yes`
        # as "no waiver" would leave the fixture comparing against an oracle
        # that cannot pass, and the operator would never learn why.
        with tempfile.TemporaryDirectory() as directory:
            corpus = Path(directory)
            (corpus / 'typo.c').write_text('int main(void) { return 0; }\n')
            (corpus / 'typo.env').write_text('LCCC_ORACLE_SELFCHECK=yes\n')
            with self.assertRaises(SystemExit) as caught:
                runner.discover([corpus])
            self.assertIn('LCCC_ORACLE_SELFCHECK', str(caught.exception))

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
