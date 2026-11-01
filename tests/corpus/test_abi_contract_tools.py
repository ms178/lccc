"""Hermetic contract tests for the gates that police GP-02.

`tests/regression/abi_attribute_matrix.py` decides whether a calling-convention
attribute is lowered, matched in GCC's own words, or refused, and
`scripts/audit_census.py` decides whether a metric may grow. Both are gates:
when they are wrong they are wrong *silently*, which is strictly worse than a
compiler bug, because a broken gate reports PASS.

So this module tests the testers, with no gcc, no lccc and no network:

  * the verdict table parsed out of the registry (shape, known variants,
    duplicate and typo rejection),
  * the oracle pin -- a disagreement is a failure on the pinned GCC and a note
    on an older one, which is what keeps the gate usable on a developer box
    while still catching oracle drift,
  * the evidence renderer that turns 'verdict disagrees with gcc' into
    `attr=[...] plain=[...]`,
  * every bounded subprocess helper (timeout, missing binary), because a gate
    that hangs or crashes names no cell, and
  * the census self-consistency invariant, which is the guard against a
    baseline that reports a metric growing while every diagnostic that
    explains the growth stays silent.

The fake compilers below are POSIX shell scripts, so the suite runs anywhere the
repository's other corpus tests do.
"""
from __future__ import annotations

from contextlib import redirect_stderr
from io import StringIO
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/regression'))
sys.path.insert(0, str(ROOT / 'scripts'))

import abi_attribute_matrix as matrix  # noqa: E402
import audit_census  # noqa: E402


def write_script(directory: Path, name: str, body: str) -> Path:
    """Create an executable fake compiler and return its path."""
    path = directory / name
    path.write_text('#!/bin/sh\n' + body)
    path.chmod(0o755)
    return path


@unittest.skipUnless(os.name == 'posix', 'fake compilers are POSIX shell scripts')
class RunBoundedTest(unittest.TestCase):
    """`run_bounded` is the only way this repository starts a compiler."""

    def test_timeout_is_a_result_not_an_exception(self):
        with tempfile.TemporaryDirectory() as tmp:
            slow = write_script(Path(tmp), 'slow', 'sleep 30\n')
            proc = matrix.run_bounded([str(slow)], timeout=0.2)
        self.assertEqual(proc.returncode, 124)
        self.assertIn('timed out', proc.stderr)

    def test_missing_binary_is_a_result_not_an_exception(self):
        proc = matrix.run_bounded(['/nonexistent/lccc-typo'], timeout=5)
        self.assertEqual(proc.returncode, 127)
        self.assertTrue(proc.stderr)

    def test_successful_run_is_passthrough(self):
        proc = matrix.run_bounded(['/bin/sh', '-c', 'echo hello'], timeout=30)
        self.assertEqual(proc.returncode, 0)
        self.assertEqual(proc.stdout.strip(), 'hello')

    def test_output_is_text(self):
        # Callers compare substrings (`'16.2' in stdout`); a bytes/str mismatch
        # is a TypeError deep inside a gate, so the mode is part of the contract.
        proc = matrix.run_bounded(['/bin/sh', '-c', 'echo 16.2'], timeout=30)
        self.assertIsInstance(proc.stdout, str)
        self.assertIn('16.2', proc.stdout)


@unittest.skipUnless(os.name == 'posix', 'fake compilers are POSIX shell scripts')
class OraclePinTest(unittest.TestCase):
    """The table is pinned to one GCC release; drift is still caught."""

    def test_version_parsing(self):
        with tempfile.TemporaryDirectory() as tmp:
            gcc = write_script(Path(tmp), 'gcc', 'echo 16.2.0\n')
            self.assertEqual(matrix.gcc_version(str(gcc)), matrix.ORACLE_GCC)
            gcc12 = write_script(Path(tmp), 'gcc12', 'echo 12.2.0\n')
            self.assertEqual(matrix.gcc_version(str(gcc12)), (12, 2))

    def test_unparseable_version_is_below_every_pin(self):
        with tempfile.TemporaryDirectory() as tmp:
            odd = write_script(Path(tmp), 'odd', 'echo unknown\n')
            self.assertEqual(matrix.gcc_version(str(odd)), (0, 0))
            broken = write_script(Path(tmp), 'broken', 'exit 3\n')
            self.assertEqual(matrix.gcc_version(str(broken)), (0, 0))
        self.assertLess((0, 0), matrix.ORACLE_GCC)

    def _compare(self, gcc_version_line: str, observation: str):
        verdicts = {('cdecl', 'x86-64'): 'GccIgnored'}
        with tempfile.TemporaryDirectory() as tmp:
            gcc = write_script(Path(tmp), 'gcc', f'echo {gcc_version_line}\n')
            with mock.patch.object(matrix, 'gcc_observation',
                                   return_value=(observation, 'silent')):
                return matrix.check_table_against_gcc(str(gcc), verdicts, False)

    def test_pinned_release_fails_on_disagreement(self):
        failures, notes = self._compare('16.2.0', 'inert')
        self.assertEqual(len(failures), 1, failures)
        self.assertEqual(failures[0][0], 'abi:cdecl/x86-64')
        self.assertTrue(any(note.startswith('oracle:') for note in notes))

    def test_newer_release_is_still_strict(self):
        failures, _ = self._compare('17.1.0', 'inert')
        self.assertEqual(len(failures), 1, failures)

    def test_older_release_becomes_a_note(self):
        failures, notes = self._compare('12.2.0', 'inert')
        self.assertEqual(failures, [])
        self.assertTrue(any('pre-oracle' in note for note in notes), notes)

    def test_agreement_never_fails(self):
        failures, _ = self._compare('16.2.0', 'ignored')
        self.assertEqual(failures, [])

    def test_implemented_convention_must_remain_honoured(self):
        verdicts = {('regparm', 'i686'): 'Implemented'}
        with tempfile.TemporaryDirectory() as tmp:
            gcc = write_script(Path(tmp), 'gcc', 'echo 16.2.0\n')
            with mock.patch.object(matrix, 'gcc_observation',
                                   return_value=('rejects', 'error: nope')):
                failures, _ = matrix.check_table_against_gcc(str(gcc), verdicts, True)
        self.assertEqual(len(failures), 1, failures)
        self.assertIn('gcc no longer honours', failures[0][1])

    def test_unknown_observation_is_reported(self):
        failures, _ = self._compare('16.2.0', 'shrugged')
        self.assertEqual(len(failures), 1, failures)
        self.assertIn('unknown gcc observation', failures[0][1])

    def test_32_bit_cells_are_skipped_without_multilib(self):
        verdicts = {('cdecl', 'i686'): 'Default'}
        with tempfile.TemporaryDirectory() as tmp:
            gcc = write_script(Path(tmp), 'gcc', 'echo 16.2.0\n')
            with mock.patch.object(matrix, 'gcc_observation') as observation:
                failures, notes = matrix.check_table_against_gcc(str(gcc), verdicts, False)
        observation.assert_not_called()
        self.assertEqual(failures, [])
        self.assertTrue(any('no 32-bit multilib' in note for note in notes), notes)


class SignatureRenderingTest(unittest.TestCase):
    """Evidence has to survive the shapes `abi_signature` actually returns."""

    def test_code_probe_signature(self):
        self.assertEqual(
            matrix.attr_pair((('rax', 'rdi'), (('rbx', 'push'),), True,
                              (('ret', '$4'),)),
                             (('rax',), (), False, ())),
            'attr=[[rax rdi] [[rbx push]] True [[ret $4]]] '
            'plain=[[rax] [] False []]')

    def test_record_probe_signature(self):
        self.assertEqual(matrix.attr_pair(('4', '8'), ('4',)),
                         'attr=[4 8] plain=[4]')

    def test_empty_signature(self):
        self.assertEqual(matrix.attr_pair((), ()), 'attr=[] plain=[]')

    def test_observation_carries_the_reason(self):
        # One listing that satisfies all three code probes; only the integer
        # probe's prologue differs between the attributed build and the
        # reference, which is exactly 'the attribute changed the ABI here'.
        def listing(prologue: str) -> str:
            return (f'f:\n pushq {prologue}\n ret\n.size f, .-f\n'
                    'h:\n addsd %xmm1, %xmm0\n ret\n.size h, .-h\n'
                    'ha:\n movq %rdi, %rax\n ret\n.size ha, .-ha\n'
                    'ps:\n movq %rdi, %rax\n ret\n.size ps, .-ps\n')

        def fake_compile(_cc, source, _flags):
            return (0, listing('%rbx'), '') if 'ms_abi' in source else \
                   (0, listing('%rbp'), '')

        with mock.patch.object(matrix, 'compile_to', side_effect=fake_compile):
            observation, evidence = matrix.gcc_observation('/bin/true', 'ms_abi', 'x86-64')
        self.assertEqual(observation, 'honours')
        self.assertIn('int:', evidence)
        self.assertIn('attr=', evidence)


class RegistryTableTest(unittest.TestCase):
    """The parsed table is the contract; its shape is asserted, not assumed."""

    def test_every_cell_is_known(self):
        verdicts = matrix.registry_verdicts()
        self.assertEqual(len(verdicts), 30, 'fifteen names x two targets')
        self.assertLessEqual(
            set(verdicts.values()),
            {'Default', 'GccIgnored', 'OnlyAvailable32Bit', 'Unsupported',
             'Implemented'})

    def test_measured_verdicts(self):
        verdicts = matrix.registry_verdicts()
        expected = {
            ('regparm', 'i686'): 'Implemented',
            ('fastcall', 'i686'): 'Implemented',
            ('ms_abi', 'x86-64'): 'Unsupported',
            ('ms_abi', 'i686'): 'Unsupported',
            ('ms_struct', 'x86-64'): 'Unsupported',
            ('gcc_struct', 'x86-64'): 'Default',
            ('sysv_abi', 'x86-64'): 'Default',
            ('callee_pop_aggregate_return', 'x86-64'): 'OnlyAvailable32Bit',
        }
        for cell, verdict in expected.items():
            self.assertEqual(verdicts[cell], verdict, cell)

    def test_register_contracts_are_refused(self):
        verdicts = matrix.registry_verdicts()
        for name in ('no_caller_saved_registers', 'no_callee_saved_registers',
                     'preserve_none'):
            for target in ('x86-64', 'i686'):
                self.assertEqual(verdicts[(name, target)], 'Unsupported',
                                 f'{name}/{target}')

    def _parse(self, body: str):
        with tempfile.TemporaryDirectory() as tmp:
            registry = Path(tmp) / 'attribute_registry.rs'
            registry.write_text('const ABI_VERDICTS: &[(&str, AbiEffect, AbiEffect)] = &[\n'
                                + body + '\n];\n')
            with mock.patch.object(matrix, 'REGISTRY', registry):
                return matrix.registry_verdicts()

    def test_duplicate_name_is_rejected(self):
        row = '    ("cdecl", AbiEffect::Default, AbiEffect::Default),\n'
        with self.assertRaises(ValueError):
            self._parse(row + row)

    def test_unknown_variant_is_rejected(self):
        with self.assertRaises(ValueError):
            self._parse('    ("cdecl", AbiEffect::Defualt, AbiEffect::Default),\n')

    def test_empty_table_is_rejected(self):
        with self.assertRaises(ValueError):
            self._parse('')

    def test_columns_are_x86_64_then_i686(self):
        parsed = self._parse('    ("regparm", AbiEffect::GccIgnored,'
                             ' AbiEffect::Implemented),\n')
        self.assertEqual(parsed[('regparm', 'x86-64')], 'GccIgnored')
        self.assertEqual(parsed[('regparm', 'i686')], 'Implemented')


@unittest.skipUnless(os.name == 'posix', 'fake compilers are POSIX shell scripts')
class LcccSideTest(unittest.TestCase):
    """`check_lccc` must accept exactly the diagnosis each verdict promises."""

    # A fake driver: FAKE_MODE picks the behaviour, the escape hatch downgrades
    # a refusal to a warning exactly as the real compiler does.
    # Arity is modelled too: the gate also drives malformed argument lists
    # through the same binary, and a fake that accepts everything would report
    # 24 phantom 'malformed attribute accepted' failures.
    FAKE = r'''
src=
for arg in "$@"; do
    case $arg in *.c) src=$arg ;; esac
done
if [ -n "$src" ]; then
    for bad in 'sysv_abi(1)' 'cdecl(1)' 'ms_abi(1)' 'regparm()' 'regparm(1,2)' \
               '((regparm))' 'gnu::regparm]]'; do
        if grep -F "$bad" "$src" >/dev/null 2>&1; then
            echo "$src:1:1: error: wrong argument count" >&2
            exit 1
        fi
    done
fi
mode=${FAKE_MODE:-silent}
if [ "$mode" = ignored ]; then
    echo "$1:1:1: warning: 'cdecl' attribute ignored [-Wattributes]" >&2
    exit 0
fi
if [ "$mode" = unsupported ]; then
    if [ -n "${CCC_ALLOW_UNSUPPORTED_ABI:-}" ]; then
        echo "$1:1:1: warning: 'ms_abi' attribute ignored (unimplemented ABI, allowed by CCC_ALLOW_UNSUPPORTED_ABI)" >&2
        exit 0
    fi
    echo "$1:1:1: error: 'ms_abi' attribute selects the Microsoft x64 calling convention, which this compiler does not implement for x86-64" >&2
    exit 1
fi
exit 0
'''

    def _driver(self, directory: Path) -> Path:
        return write_script(directory, 'lccc', self.FAKE)

    def setUp(self):
        # These tests pin the *cell* logic; the sentinel builds and links real
        # objects, cannot work against a shell stub, and is covered by
        # SentinelWiringTest.
        patch = mock.patch.object(matrix, 'check_sentinels', return_value=([], []))
        patch.start()
        self.addCleanup(patch.stop)

    def test_default_must_be_silent(self):
        with tempfile.TemporaryDirectory() as tmp:
            driver = self._driver(Path(tmp))
            failures, _ = matrix.check_lccc({('sysv_abi', 'x86-64'): 'Default'}, driver)
        self.assertEqual(failures, [])

    def test_ignored_must_reproduce_wording(self):
        with tempfile.TemporaryDirectory() as tmp:
            driver = self._driver(Path(tmp))
            with mock.patch.dict(matrix.ENV, {'FAKE_MODE': 'ignored'}):
                failures, _ = matrix.check_lccc({('cdecl', 'x86-64'): 'GccIgnored'}, driver)
        self.assertEqual(failures, [])

    def test_ignored_without_the_warning_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            driver = self._driver(Path(tmp))
            with mock.patch.dict(matrix.ENV, {'FAKE_MODE': 'silent'}):
                failures, _ = matrix.check_lccc({('cdecl', 'x86-64'): 'GccIgnored'}, driver)
        cell = [f for f in failures if f[0] == 'abi:cdecl/x86-64']
        self.assertEqual(len(cell), 1, failures)
        self.assertIn('wording not reproduced', cell[0][1])

    def test_refusal_and_escape_hatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            driver = self._driver(Path(tmp))
            with mock.patch.dict(matrix.ENV, {'FAKE_MODE': 'unsupported'}):
                failures, _ = matrix.check_lccc({('ms_abi', 'x86-64'): 'Unsupported'}, driver)
        self.assertEqual(failures, [])

    def test_refusal_that_still_compiles_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            driver = self._driver(Path(tmp))
            with mock.patch.dict(matrix.ENV, {'FAKE_MODE': 'silent'}):
                failures, _ = matrix.check_lccc({('ms_abi', 'x86-64'): 'Unsupported'}, driver)
        # The fake accepts everything, so the cell reports both the missing
        # refusal and the (therefore unfalsifiable) escape hatch; the first
        # finding is the one that matters.
        cell = [f for f in failures if f[0] == 'abi:ms_abi/x86-64']
        self.assertTrue(cell, failures)
        self.assertIn('accepted silently', cell[0][1])

    def test_missing_driver_skips_without_failing(self):
        verdicts = matrix.registry_verdicts()
        failures, notes = matrix.check_lccc(verdicts, Path('/nonexistent/lccc'))
        self.assertEqual(failures, [])
        self.assertTrue(any('not built' in note for note in notes), notes)


class CensusConsistencyTest(unittest.TestCase):
    """The ratchet must name corruption instead of looking like a regression."""

    def test_consistent_report(self):
        self.assertIsNone(audit_census.check_self_consistency(
            {'knob_literals': 2, 'knob_names': ['CCC_A', 'CCC_B']}))

    def test_inconsistent_report(self):
        problem = audit_census.check_self_consistency(
            {'knob_literals': 2, 'knob_names': ['CCC_A']})
        self.assertIsNotNone(problem)
        self.assertIn('disagrees', problem)

    def test_partial_report_is_not_a_failure(self):
        self.assertIsNone(audit_census.check_self_consistency({'files': 3}))

    def test_growth_line_names_the_new_knob(self):
        rendered = audit_census.describe_growth(
            'knob_literals',
            {'knob_names': ['CCC_A']},
            {'knob_names': ['CCC_A', 'CCC_B']})
        self.assertEqual(rendered, ' [CCC_B]')

    def test_growth_line_truncates_many_names(self):
        new = [f'CCC_{i}' for i in range(10)]
        rendered = audit_census.describe_growth(
            'knob_literals', {'knob_names': []}, {'knob_names': new})
        self.assertTrue(rendered.endswith(', ...]'), rendered)

    def test_growth_line_is_empty_without_names(self):
        self.assertEqual(audit_census.describe_growth(
            'fns_over_300', {'knob_names': []}, {'knob_names': []}), '')

    def test_update_refuses_an_inconsistent_report(self):
        with mock.patch.object(audit_census, 'BASELINE', Path('/nonexistent/baseline.json')), \
                redirect_stderr(StringIO()):
            code = audit_census.cmd_update(
                {'knob_literals': 2, 'knob_names': ['CCC_A']})
        self.assertEqual(code, 1)

    def test_check_refuses_an_inconsistent_baseline(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / 'baseline.json'
            baseline.write_text('{"knob_literals": 1, "knob_names": ["CCC_A", "CCC_B"]}')
            with mock.patch.object(audit_census, 'BASELINE', baseline), \
                    redirect_stderr(StringIO()):
                code = audit_census.cmd_check(
                    {'knob_literals': 1, 'knob_names': ['CCC_A', 'CCC_B']})
        self.assertEqual(code, 1)


class SentinelWiringTest(unittest.TestCase):
    """The sentinel must use the pinned oracle, not whatever `gcc` resolves to."""

    def test_missing_tools_skip_cleanly(self):
        failures, notes = matrix.check_sentinels(Path('/nonexistent/lccc'),
                                                 '/nonexistent/gcc')
        self.assertEqual(failures, [])
        self.assertTrue(any('skipped' in note for note in notes), notes)

    def test_env_gcc_is_preferred_over_path(self):
        with mock.patch.dict(os.environ, {'GCC': '/pinned/gcc-16'}, clear=False), \
                mock.patch.object(matrix.Path, 'exists', return_value=True):
            captured = {}

            def fake_run(cmd, timeout, **kwargs):
                captured.setdefault('cmd', cmd)
                return subprocess.CompletedProcess(cmd, 1, '', 'no')

            with mock.patch.object(matrix, 'run_bounded', side_effect=fake_run):
                matrix.check_sentinels(Path('/pinned/lccc'))
        self.assertEqual(captured['cmd'][0], '/pinned/gcc-16')


if __name__ == '__main__':
    unittest.main()
