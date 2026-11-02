"""Every evidence section survives the real per-step Checks notice budget."""
from contextlib import redirect_stdout
import copy
import io
import json
from pathlib import Path
import tempfile
import unittest

try:
    import yaml
except ModuleNotFoundError as exc:  # pragma: no cover - host dependency
    # CI installs python3-yaml (scripts/ci_ubuntu_chroot.sh PACKAGES), so this
    # only fires on a developer box.  Importing the workflow contracts without a
    # parser is impossible, and failing during collection reports the missing
    # package as an unrelated test error; a skip states the real cause once.
    raise unittest.SkipTest(
        "PyYAML is required to read the CI workflow contracts "
        "(install python3-yaml, or `pip install pyyaml`)"
    ) from exc

from tests.corpus.test_compiler_comparison_reports import load, report, summary

publisher = load('publish_compiler_comparison')
ROOT = Path(__file__).resolve().parents[2]


def native_report():
    data = report()
    for label in ['clang', 'pr835']:
        data['compilers'].append({'key': label})
        for row in data['benchmarks']:
            for field in ['correctness', 'runs', 'runtime', 'ratios_from_lccc']:
                row[field][label] = copy.deepcopy(row[field]['prior'])
    for row in data['benchmarks']:
        row['warmups'] = {key['key']: [{'ok': True, 'stdout': 'correct\n'}]
                          for key in data['compilers']}
        for ratio in row['ratios_from_lccc'].values():
            ratio.update(median=1.1234567890123456,
                         median_ci95_bootstrap=[1.0987654321098765, 1.2345678901234567])
    return data


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data))


def invoke(root, section):
    output = io.StringIO()
    with redirect_stdout(output):
        status = publisher.main(['--root', str(root), '--section', section])
    notices = []
    for line in output.getvalue().splitlines():
        if line.startswith('::notice '):
            head, body = line[2:].split('::', 1)
            notices.append((head.removeprefix('notice title='), json.loads(body)))
            if len(body.encode()) > 3800:
                raise AssertionError('oversized notice')
    if len(notices) > 10:
        raise AssertionError('Actions would discard notices')
    return status, notices, output.getvalue()


def semantic_report(root):
    labels = ['gcc', 'final', 'gcc16-optimized', 'main', 'pr835', 'prior']
    rows = []
    for name, *_ in publisher.cases():
        arms = {}
        for label in labels:
            passed = label in ['gcc', 'final', 'prior']
            arms[label] = {'pass': passed, 'build': {'status': 0},
                           'run': {'status': 0 if passed else 1}}
        rows.append({'id': name, 'arms': arms})
    data = {'complete': True, 'control_policy': 'synthetic independent control fixture',
            'compilers': {label: {'sha256': 'a' * 64} for label in labels},
            'required': ['final'], 'cases': rows,
            'summary': {label: {'pass': sum(row['arms'][label]['pass'] for row in rows),
                                'fail': sum(not row['arms'][label]['pass'] for row in rows)}
                        for label in labels}}
    for label in ['final', 'main', 'pr835', 'prior']:
        (root / (label + '.commit')).write_text('a' * 40)
    write(root / 'witnesses/index.json', data)
    return data


class EvidencePublication(unittest.TestCase):
    def test_all_native_rows_and_five_arms_fit_a_single_step(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write(root / 'native.json', native_report())
            status, notices, output = invoke(root, 'native')
            self.assertEqual(status, 0, output)
            rows = [row for title, data in notices if title.startswith('compiler comparison rows')
                    for row in data]
            self.assertEqual({row['name'] for row in rows}, {case.name for case in summary.bench.BENCHMARKS})
            self.assertEqual(len(rows), len(summary.bench.BENCHMARKS))
            self.assertEqual(len(rows[0]['ratios']), 4)
            self.assertEqual(len(rows[0]['ratios']['pr835']), len(summary.COMPACT_RATIO_COLUMNS))

    def test_o3_is_a_complete_correctness_inventory_not_timing(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            data = native_report()
            data['settings']['skip_perf'] = True
            for row in data['benchmarks']:
                row['warmups'] = {}
                row['runs'] = {key: runs[:1] for key, runs in row['runs'].items()}
                row['runtime'] = {key: {'wall': None} for key in row['runs']}
                row['ratios_from_lccc'] = {}
            write(root / 'correctness-o3.json', data)
            status, notices, output = invoke(root, 'correctness-o3')
            self.assertEqual(status, 0, output)
            self.assertIs(notices[0][1]['timing_claim'], False)
            rows = [row for title, data in notices if title.startswith('compiler comparison rows')
                    for row in data]
            self.assertEqual(len(rows), len(summary.bench.BENCHMARKS))
            self.assertTrue(all(set(row) == {'name', 'correct'} for row in rows))

    def test_all_65_semantic_failure_rows_survive_without_failing_good_required_arms(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            semantic_report(root)
            status, notices, output = invoke(root, 'semantic')
            self.assertEqual(status, 0, output)
            rows = [row for title, data in notices if title == 'semantic witness failures' for row in data]
            self.assertEqual(len(rows), 65)
            self.assertEqual({row['case'] for row in rows}, {case[0] for case in publisher.cases()})

    def test_required_failure_and_incomplete_or_duplicate_semantic_cases_fail(self):
        for mutation in ['required', 'missing', 'duplicate', 'identity']:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                data = semantic_report(root)
                if mutation == 'required':
                    # Cached summary still says pass: raw required-case failure wins.
                    data['cases'][0]['arms']['final']['pass'] = False
                elif mutation == 'missing':
                    data['cases'].pop()
                elif mutation == 'duplicate':
                    data['cases'][-1] = copy.deepcopy(data['cases'][0])
                else:
                    (root / 'final.commit').unlink()
                write(root / 'witnesses/index.json', data)
                self.assertEqual(invoke(root, 'semantic')[0], 1)

    def test_static_and_simulated_counters_keep_every_alias_in_separate_sections(self):
        names = [case.name for case in summary.bench.BENCHMARKS]
        for section in publisher.SECTIONS[3:]:
            with self.subTest(section=section), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                if section.startswith('codegen-'):
                    path = root / 'result.json'
                    record = {'key': 'lccc', 'instructions': 100, 'loads': 10, 'stores': 5,
                              'branches': 3, 'vectors': 7, 'calls': 2, 'spills': 4}
                    write(path, [{'errors': [], 'records': [record]}])
                    entries = {name: {'exit_code': 0, 'report': str(path)} for name in names}
                else:
                    metrics = {'ir_ref': 100, 'ir_mine': 110, 'ir_ratio': 1.1,
                               'i1_misses': [1, 2], 'd1_misses': [3, 4], 'll_misses': [5, 6],
                               'conditional_mispredicts': [7, 8], 'indirect_mispredicts': [9, 10]}
                    entries = {name: {'metrics': metrics} for name in names}
                write(root / section / 'index.json', {'complete': True, 'selected': names,
                      'results': entries, 'failures': [], 'metric': 'synthetic schema fixture; not runtime'})
                status, notices, output = invoke(root, section)
                self.assertEqual(status, 0, output)
                self.assertTrue(notices[0][1]['full_registry_valid'])
                rows = [row for _, data in notices[1:] for row in data]
                self.assertEqual(len(rows), len(names))
                self.assertEqual({row['name'] for row in rows}, set(names))

    def test_missing_sections_and_partial_instrumentation_never_succeed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for section in publisher.SECTIONS:
                with self.subTest(section=section):
                    status, _, output = invoke(root, section)
                    self.assertEqual(status, 1)
                    self.assertIn('::error ', output)
            write(root / 'callgrind/index.json', {'complete': False, 'selected': ['fib'],
                  'results': {}, 'failures': [], 'metric': 'incomplete synthetic fixture'})
            status, notices, _ = invoke(root, 'callgrind')
            self.assertEqual(status, 1)
            self.assertFalse(notices[0][1]['full_registry_valid'])

    def test_notice_budget_overflow_emits_no_successful_prefix(self):
        def oversized():
            for number in range(11):
                summary.notice('synthetic', {'n': number})
        output = io.StringIO()
        with redirect_stdout(output), self.assertRaises(ValueError):
            summary.bounded_notices(oversized)
        self.assertEqual(output.getvalue(), '')

    def test_each_section_has_its_own_unconditional_workflow_step(self):
        workflow = yaml.load((ROOT / '.github/workflows/compiler-comparison.yml').read_text(), Loader=yaml.BaseLoader)
        steps = workflow['jobs']['compiler-comparison']['steps']
        publish = [step for step in steps if 'scripts/publish_compiler_comparison.py' in step.get('run', '')]
        self.assertEqual(len(publish), len(publisher.SECTIONS))
        self.assertEqual({step['run'].split('--section ', 1)[1] for step in publish}, set(publisher.SECTIONS))
        for step in publish:
            self.assertEqual(step['if'], 'always()')
            self.assertNotIn('continue-on-error', step)
        boundaries = {
            'semantic': 'Correctness-gated paired native comparison of all registered shapes',
            'native': 'Screen all shapes at O3 without making a second timing claim',
            'correctness-o3': 'Registry-aware local codegen oracles for each exact compiler',
            'codegen-final': 'Bounded Callgrind comparison of every registered shape',
            'codegen-main': 'Bounded Callgrind comparison of every registered shape',
            'codegen-pr835': 'Bounded Callgrind comparison of every registered shape',
            'codegen-prior': 'Bounded Callgrind comparison of every registered shape',
            'callgrind': 'Independent GCC control for the bounded instrumentation sizes',
            'callgrind-gcc-control': 'Retain raw commands, outputs, binaries and counter evidence',
        }
        for section, boundary in boundaries.items():
            produced = next(i for i, step in enumerate(steps)
                            if step.get('run', '').endswith('--section ' + section))
            following = next(i for i, step in enumerate(steps) if step.get('name') == boundary)
            self.assertLess(produced, following, section)


if __name__ == '__main__':
    unittest.main()
