"""Fail-closed coverage, provenance and launch-floor rules for audit evidence."""
from contextlib import redirect_stdout
import copy
import importlib.util
import io
import json
from pathlib import Path
from types import SimpleNamespace
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'scripts' / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


summary = load('benchmark_comparison_report')
witness = load('compare_compiler_witnesses')


def report():
    rows = []
    for case in summary.bench.BENCHMARKS:
        rows.append({'name': case.name,
                     'correctness': {key: {'matches_baseline': True, 'stable_within_compiler': True}
                                     for key in ['lccc', 'gcc', 'prior']},
                     'runs': {key: [{'ok': True, 'stdout': 'correct\n', 'wall_s': .09 if key == 'lccc' else .1} for _ in range(9)] for key in ['lccc', 'gcc', 'prior']},
                     'runtime': {key: {'wall': {'median': .09 if key == 'lccc' else .1}} for key in ['lccc', 'gcc', 'prior']},
                     'ratios_from_lccc': {key: {'n': 9, 'median': .9} for key in ['gcc', 'prior']}})
    for row in rows:
        row['rounds'] = [{'round': index, 'order': ['lccc', 'gcc', 'prior'],
                          'runs': {arm: row['runs'][arm][index] for arm in ['lccc', 'gcc', 'prior']}}
                         for index in range(9)]
    return {'complete': True, 'benchmarks': rows, 'compilers': [{'key': key} for key in ['lccc', 'gcc', 'prior']],
            'settings': {'skip_perf': False}}


class ComparisonReport(unittest.TestCase):
    def test_full_correct_registry_is_required_and_never_deduplicated(self):
        data = report()
        self.assertEqual(summary.summarize(data)['comparisons']['prior']['timing_eligible'], 45)
        for mutate in [lambda rows: rows.pop(), lambda rows: rows.append(copy.deepcopy(rows[0])),
                       lambda rows: rows[0].update(name='unknown')]:
            bad = copy.deepcopy(data)
            mutate(bad['benchmarks'])
            with self.assertRaises(ValueError):
                summary.summarize(bad)

    def test_floor_rows_remain_visible_but_never_enter_primary_ratio(self):
        data = report()
        data['benchmarks'][0]['runtime']['lccc']['wall']['median'] = .001
        result = summary.summarize(data)
        self.assertEqual(result['comparisons']['prior']['correct'], 45)
        self.assertEqual(result['comparisons']['prior']['timing_eligible'], 44)
        self.assertFalse(result['rows'][0]['ratios']['prior']['eligible'])

    def test_wrong_failed_unstable_and_short_arms_are_not_performance_evidence(self):
        for mutate in [
            lambda row: row['correctness']['prior'].update(matches_baseline=False),
            lambda row: row['correctness']['prior'].update(stable_within_compiler=False),
            lambda row: row['runs']['prior'].append({'ok': False}),
            lambda row: row['ratios_from_lccc']['prior'].update(n=1),
            lambda row: row['runs']['prior'].clear(),
            lambda row: row['runs']['prior'].pop(),
            lambda row: row['correctness']['gcc'].update(matches_baseline=False),
        ]:
            data = report()
            mutate(data['benchmarks'][0])
            self.assertEqual(summary.summarize(data)['comparisons']['prior']['timing_eligible'], 44)

    def test_raw_warmups_and_samples_override_cached_success_flags(self):
        for location, arm, observation in [
            ('warmups', 'prior', {'ok': True, 'stdout': 'wrong\n'}),
            ('warmups', 'prior', {'ok': False}),
            ('warmups', 'gcc', {'ok': True, 'stdout': 'wrong\n'}),
            ('warmups', 'gcc', {'ok': False}),
            ('runs', 'prior', {'ok': True, 'stdout': 'wrong\n'}),
            ('runs', 'gcc', {'ok': True, 'stdout': 'wrong\n'}),
        ]:
            with self.subTest(location=location, arm=arm, observation=observation):
                data = report()
                data['benchmarks'][0].setdefault(location, {}).setdefault(arm, []).append(observation)
                result = summary.summarize(data)
                self.assertEqual(result['comparisons']['prior']['timing_eligible'], 44)
                self.assertFalse(result['rows'][0]['correct']['prior'])

    def test_codegen_counts_preserve_aliases_and_name_stack_references_truthfully(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'result.json'
            record = {'key': 'lccc', 'instructions': 100, 'loads': 10, 'stores': 5,
                      'branches': 3, 'vectors': 7, 'calls': 2, 'spills': 4}
            path.write_text(json.dumps([{'errors': [], 'records': [record]}]))
            entry = {'exit_code': 0, 'report': str(path)}
            index = {'results': {'linux_find_bit': entry, 'linux_find_bit_scaled': entry,
                                 'failed': {'exit_code': 1, 'report': 'must-not-read'}}}
            rows = summary.codegen_rows(index)
            self.assertEqual([row['name'] for row in rows], ['linux_find_bit', 'linux_find_bit_scaled'])
            self.assertEqual(rows[0]['stack_refs'], 4)
            self.assertNotIn('spills', rows[0])
            self.assertEqual(len(rows[0]['sha256']), 64)
            self.assertEqual(rows[0]['instructions'], 100)

    def test_invalid_successful_codegen_records_cannot_publish_counts(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'result.json'
            index = {'results': {'shape': {'exit_code': 0, 'report': str(path)}}}
            good = {'key': 'lccc', 'instructions': 1, 'loads': 0, 'stores': 0,
                    'branches': 0, 'vectors': 0, 'calls': 0, 'spills': 0}
            for data in ([], [{'errors': ['failure'], 'records': [good]}],
                         [{'records': []}], [{'records': [good, good]}],
                         [{'records': [{**good, 'instructions': -1}]}],
                         [{'records': [{**good, 'spills': None}]}]):
                with self.subTest(data=data):
                    path.write_text(json.dumps(data))
                    with self.assertRaises(ValueError):
                        summary.codegen_rows(index)

    def test_correctness_only_never_yields_a_timing_claim(self):
        data = report()
        data['settings']['skip_perf'] = True
        result = summary.summarize(data)['comparisons']['prior']
        self.assertEqual(result['timing_eligible'], 0)
        self.assertIsNone(result['geometric_mean_ratio'])

    def test_nonfinite_or_nonpositive_ratios_are_errors(self):
        for value in [float('nan'), float('inf'), 0, -1]:
            data = report()
            data['benchmarks'][0]['ratios_from_lccc']['prior']['median'] = value
            with self.assertRaises(ValueError):
                summary.summarize(data)

    def test_notice_is_bounded_and_escapes_workflow_metacharacters(self):
        out = io.StringIO()
        with redirect_stdout(out):
            summary.notice('example', {'value': '100%\n'})
        self.assertIn('%25', out.getvalue())
        with self.assertRaises(ValueError):
            summary.notice('oversized', {'value': 'x' * 4000})


    def test_notice_rows_keeps_all_rows_within_the_api_budget(self):
        rows = [{'id': value, 'payload': 'x' * 600} for value in range(45)]
        out = io.StringIO()
        with redirect_stdout(out):
            summary.notice_rows('rows', rows)
        notices = out.getvalue().splitlines()
        self.assertLess(len(notices), 10)
        recovered = []
        for line in notices:
            message = line.split('::', 2)[2]
            self.assertLessEqual(len(message.encode()), 3800)
            recovered.extend(json.loads(message))
        self.assertEqual(recovered, rows)


class HostedComparison(unittest.TestCase):
    def workflow(self):
        import yaml
        return yaml.load((ROOT / '.github/workflows/compiler-comparison.yml').read_text(), Loader=yaml.BaseLoader)

    def test_research_is_explicitly_opted_in_without_widening_token_permissions(self):
        data = self.workflow()
        job = data['jobs']['compiler-comparison']
        self.assertIn("github.event_name == 'workflow_dispatch' && inputs.audit_compare", job['if'])
        self.assertIn("contains(github.event.pull_request.labels.*.name, 'compiler-comparison')", job['if'])
        self.assertEqual(data['permissions'], {'contents': 'read'})
        self.assertIn('labeled', data['on']['pull_request']['types'])
        self.assertEqual(data['on']['workflow_dispatch']['inputs']['audit_compare']['default'], 'false')

    def test_failures_do_not_censor_independent_measurements_or_become_success(self):
        job = self.workflow()['jobs']['compiler-comparison']
        self.assertNotIn('continue-on-error', job)
        steps = job['steps']
        measures = [step for step in steps if step.get('if', '').startswith('${{ !cancelled()')]
        self.assertEqual(len(measures), 6)
        for step in measures:
            self.assertIn("steps.comparison-build.outcome == 'success'", step['if'])
            self.assertNotIn('continue-on-error', step)
            self.assertIn('set -euo pipefail', step['run'])
        runs = '\n'.join(step.get('run', '') for step in steps)
        self.assertIn('git archive "$sha"', runs)
        self.assertNotIn('git checkout', runs)
        self.assertNotIn('git switch', runs)
        self.assertIn('--ref gcc-16 --gcc-include-command gcc-16', runs)
        self.assertIn("--oracles ''", runs)
        self.assertEqual(steps[-1]['if'], 'always()')


class NativeWitnesses(unittest.TestCase):
    def test_matrix_is_unique_and_semantics_are_explicit(self):
        cases = list(witness.cases())
        self.assertEqual(len(cases), 65)
        self.assertEqual(len({case[0] for case in cases}), 65)
        for _, source, flags, _ in cases:
            self.assertIn('-std=gnu17', flags)
            self.assertTrue((ROOT / 'tests/regression' / (source + '.c')).is_file())
            if source.startswith('wrapv_'):
                self.assertIn('-fwrapv', flags)
            if source.startswith('fenv_'):
                self.assertIn('-frounding-math', flags)

    def test_process_failure_and_timeout_are_retained_not_skipped(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            run = witness.execute([sys.executable, '-c', 'print("raw");exit(17)'], root / 'failure', {}, 2)
            self.assertEqual(run['status'], 17)
            self.assertEqual((root / 'failure/stdout').read_bytes(), b'raw\n')
            run = witness.execute([sys.executable, '-c', 'import time;time.sleep(1)'], root / 'timeout', {}, .01)
            self.assertEqual(run['status'], 'error')
            self.assertIn('timed out', run['stderr_tail'])

    def test_timed_out_output_is_preserved_verbatim(self):
        error = subprocess.TimeoutExpired(['compiler'], 1, output=b'partial output', stderr=b'partial error')
        with tempfile.TemporaryDirectory() as temp, \
             mock.patch.object(witness.subprocess, 'run', side_effect=error):
            result = witness.execute(['compiler'], Path(temp), {}, 1)
            self.assertEqual(result['status'], 'error')
            self.assertEqual((Path(temp) / 'stdout').read_bytes(), b'partial output')
            self.assertTrue((Path(temp) / 'stderr').read_bytes().startswith(b'partial error'))

    def test_header_discovery_failure_publishes_an_incomplete_failure(self):
        with tempfile.TemporaryDirectory() as temp, \
             mock.patch.object(witness.subprocess, 'run', side_effect=OSError('synthetic discovery failure')), \
             redirect_stdout(io.StringIO()):
            status = witness.main(['--gcc', sys.executable, '--compiler', 'final=' + sys.executable,
                                   '--require', 'final', '--out', temp])
            self.assertEqual(status, 1)
            data = json.loads((Path(temp) / 'index.json').read_text())
            self.assertFalse(data['complete'])
            self.assertIn('synthetic discovery failure', data['error'])

    def test_zero_exit_without_executable_cannot_certify_a_control(self):
        with tempfile.TemporaryDirectory() as temp, \
             mock.patch.object(witness, 'cases', return_value=[next(witness.cases())]), \
             mock.patch.object(witness, 'execute', return_value={'status': 0}), \
             mock.patch.object(witness.subprocess, 'run', return_value=SimpleNamespace(returncode=0, stdout=temp)), \
             redirect_stdout(io.StringIO()):
            status = witness.main(['--gcc', sys.executable, '--compiler', 'final=' + sys.executable,
                                   '--require', 'final', '--out', temp])
            self.assertEqual(status, 1)
            data = json.loads((Path(temp) / 'index.json').read_text())
            self.assertEqual(data['summary']['final']['pass'], 0)

    @unittest.skipUnless(shutil.which('gcc'), 'requires host GCC for the test-oracle control')
    def test_native_gcc_control_and_truthfully_named_candidate_match(self):
        with tempfile.TemporaryDirectory() as temp, \
             mock.patch.object(witness, 'cases', return_value=[next(case for case in witness.cases() if case[0] == 'macro_literal_payload/O2')]), \
             redirect_stdout(io.StringIO()):
            status = witness.main(['--gcc', 'gcc', '--compiler', 'control=gcc',
                                   '--require', 'control', '--out', temp])
            self.assertEqual(status, 0)
            data = json.loads((Path(temp) / 'index.json').read_text())
            self.assertTrue(data['complete'])
            self.assertEqual(data['summary']['control']['pass'], 1)
            self.assertIsNotNone(data['compilers']['control']['sha256'])
            row = data['cases'][0]
            self.assertIn('-O2', row['arms']['control']['build']['command'])
            self.assertIn('-O0', row['arms']['gcc']['build']['command'])
            self.assertNotIn('-O2', row['arms']['gcc']['build']['command'])


if __name__ == '__main__':
    unittest.main()
