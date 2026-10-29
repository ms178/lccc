#!/usr/bin/env python3
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('hybrid', ROOT / 'scripts/benchmark_hybrid_perf.py')
hybrid = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hybrid)


class HybridTests(unittest.TestCase):
    def test_cpu_sets_and_pinning(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for name, cpus in [('cpu_core', '0-3,8-11'), ('cpu_atom', '4-7')]:
                (root / name).mkdir()
                (root / name / 'cpus').write_text(cpus)
            classes = hybrid.choose_cpus(root, {2, 6}, None, None)
            self.assertEqual([(c['class'], c['cpu']) for c in classes], [('P', 2), ('E', 6)])
            with self.assertRaises(ValueError):
                hybrid.choose_cpus(root, {2, 6}, 6, None)
            self.assertEqual(hybrid.choose_cpus(root, {2}, None, None)[1]['cpu'], None)
            (root / 'cpu_atom/cpus').write_text('2-7')
            with self.assertRaises(ValueError):
                hybrid.choose_cpus(root, {2, 6}, None, None)

    def test_generic_does_not_claim_p_or_e(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(hybrid.choose_cpus(Path(d), {8}, None, None)[0]['class'], 'generic (P/E unknown)')
            with self.assertRaises(ValueError):
                hybrid.choose_cpus(Path(d), {8}, 8, None)

    def test_counter_validity(self):
        def rows(pct='100.00', value='123'):
            return ''.join(f'{value};;cpu_core/{event}/u;1000;{pct};;\n' for event in
                           ('cycles', 'instructions', 'branches', 'branch-misses'))
        self.assertTrue(hybrid.counter_status(rows(), 'cpu_core')[0])
        for data in (rows('20'), rows(value='<not supported>'), rows() + rows(), '', rows('nan'), rows(value='0')):
            self.assertFalse(hybrid.counter_status(data, 'cpu_core')[0])

    def test_paired_summary_excludes_failure_and_zero_denominator(self):
        def obs(mult, zero_branches=False):
            lines = ''.join(f'{0 if zero_branches and event == "branches" else 100 * mult};;{event}:u;1000;100.00;;\n'
                            for event in ('cycles', 'instructions', 'branches', 'branch-misses'))
            return {'ok': True, 'perf_csv': lines}
        pairs = [{'lccc': obs(2), 'gcc': obs(1)} for _ in range(9)]
        summary = hybrid.summarize_perf(pairs, None, 42)
        self.assertTrue(summary['complete'])
        self.assertEqual(summary['metrics']['cycles']['median_ratio'], 2.0)
        self.assertEqual(summary['metrics']['cycles']['median_ci95_bootstrap'], [2.0, 2.0])
        self.assertIsNone(hybrid.summarize_perf(pairs[:1], None, 42)['metrics']['cycles']['median_ci95_bootstrap'])
        broken = list(pairs); broken[0] = {'lccc': {'ok': False}, 'gcc': obs(1)}
        self.assertFalse(hybrid.summarize_perf(broken, None, 42)['complete'])
        broken[0] = {'lccc': obs(2), 'gcc': obs(1, True)}
        self.assertIsNone(hybrid.summarize_perf(broken, None, 42)['metrics']['branches']['ratio'])

    def test_preflight_failure_does_not_launch_corpus(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            cpu = min(os.sched_getaffinity(0))
            args = ['--lccc', '/usr/bin/gcc', '--only', 'gzip_crc32', '--output', str(root / 'r.txt'),
                    '--artifacts', str(root / 'evidence')]
            failed = {'ok': False, 'reason': 'counter unavailable'}
            with mock.patch.object(hybrid, 'choose_cpus', return_value=[{'class': 'generic', 'cpu': cpu, 'pmu': None}]), \
                 mock.patch.object(hybrid, 'run_perf', return_value=failed), \
                 mock.patch.object(hybrid.subprocess, 'run') as run, \
                 mock.patch.object(hybrid.shutil, 'which', side_effect=lambda name: '/usr/bin/' + name):
                run.return_value.stdout = 'mock perf version'
                self.assertEqual(hybrid.main(args), 1)
                self.assertEqual(run.call_count, 1)  # perf --version only; no corpus launched
            self.assertIn('RESULT: FAILED/INCOMPLETE', (root / 'r.txt').read_text())
            self.assertIn('counter unavailable', (root / 'r.txt').read_text())

    def test_source_delta_retains_tracked_patch_and_warns_untracked(self):
        if not __import__('shutil').which('git'):
            self.skipTest('git unavailable')
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / 'src'; root.mkdir()
            evidence = Path(td) / 'evidence'; evidence.mkdir()
            subprocess.run(['git', 'init', '-q', str(root)], check=True)
            (root / 'code.c').write_text('old\n')
            subprocess.run(['git', '-C', str(root), 'add', 'code.c'], check=True)
            subprocess.run(['git', '-C', str(root), '-c', 'user.name=test', '-c', 'user.email=test@example.invalid',
                            'commit', '-qm', 'base'], check=True)
            (root / 'code.c').write_text('new\n')
            (root / 'extra.c').write_text('untracked\n')
            result = hybrid.capture_source_delta(root, evidence)
            self.assertTrue(result['available'], result)
            self.assertEqual(result['untracked_files'], ['extra.c'])
            self.assertIn('not included', result['warning'])
            data = (evidence / 'tracked-source-diff.patch').read_bytes()
            self.assertIn(b'+new', data)
            self.assertEqual(result['tracked_diff_sha256'], __import__('hashlib').sha256(data).hexdigest())
            self.assertFalse(hybrid.capture_source_delta(Path(td), evidence)['available'])

    def test_perf_output_limit_fails_closed(self):
        cpu = min(os.sched_getaffinity(0))
        with tempfile.TemporaryDirectory() as td:
            fake = Path(td) / 'perf'
            fake.write_text('#!/usr/bin/env python3\nimport sys\nsys.stdout.write("x" * (2 * 1024 * 1024))\n')
            fake.chmod(0o755)
            result = hybrid.run_perf(str(fake), cpu, None, Path('/bin/true'), 15, None)
            self.assertFalse(result['ok'])
            self.assertEqual(result['execution_failure'], 'output-limit')
            self.assertIn('output-limit', result['reason'])

    def test_fails_closed_on_missing_pmu(self):
        if not os.path.isfile('/usr/bin/perf'):
            self.skipTest('perf unavailable')
        cpu = min(os.sched_getaffinity(0))
        result = hybrid.run_perf('/usr/bin/perf', cpu, None, Path('/bin/true'), 15, None)
        if not result['ok']:
            self.assertTrue(result['reason'])

    def test_end_to_end_with_fake_perf_and_real_corpus(self):
        if not all(__import__('shutil').which(x) for x in ('gcc', 'taskset', 'objdump')):
            self.skipTest('compiler/tool unavailable')
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            fake = root / 'perf'
            fake.write_text('''#!/usr/bin/env python3
import subprocess, sys
if sys.argv[1:] == ['--version']:
    print('mock perf; not hardware evidence'); sys.exit(0)
a = sys.argv[1:]
with open(a[a.index('-o') + 1], 'w') as f:
    for e in ('cycles', 'instructions', 'branches', 'branch-misses'):
        f.write('123;;%s:u;10000;100.00;;\\n' % e)
sys.exit(subprocess.run(a[a.index('--') + 1:]).returncode)
''')
            fake.chmod(0o755)
            cpu = min(os.sched_getaffinity(0))
            args = ['--lccc', '/usr/bin/gcc', '--gcc', '/usr/bin/gcc', '--only', 'gzip_crc32',
                    '--reps', '1', '--warmup', '0', '--perf-reps', '9',
                    '--output', str(root / 'report.txt'), '--artifacts', str(root / 'evidence')]
            with mock.patch.dict(os.environ, {'PATH': str(root) + os.pathsep + os.environ['PATH']}), \
                 mock.patch.object(hybrid, 'choose_cpus', return_value=[{'class': 'generic', 'cpu': cpu, 'pmu': None}]):
                self.assertEqual(hybrid.main(args), 0)
            # A successful perf process is not enough: the measured program's
            # stdout must still match the previously verified corpus output.
            mismatch = hybrid.run_perf(str(fake), cpu, None, Path('/bin/echo'), 15, 'bad-hash')
            self.assertFalse(mismatch['ok'])
            self.assertIn('stdout differs', mismatch['reason'])
            report = (root / 'report.txt').read_text()
            self.assertIn('RESULT: COMPLETE', report)
            self.assertIn('gzip_crc32', report)
            self.assertIn('mock perf; not hardware evidence', report)
            self.assertIn('benchmark.bin', report)
            rows = [json.loads(line) for line in report.splitlines() if line.startswith('{"command":')]
            self.assertEqual(len(rows), 18)
            for r in range(0, 8, 2):
                first = [row['compiler'] for row in rows if row['round'] == r]
                second = [row['compiler'] for row in rows if row['round'] == r + 1]
                self.assertEqual(second, list(reversed(first)))
            self.assertIn('PMU paired summary:', report)
            self.assertIn('"median_ci95_bootstrap": [1.0, 1.0]', report)

    def test_cpulist_rejects_malformed(self):
        for text in ('', '1-0', 'x', '1--2'):
            with self.assertRaises(ValueError):
                hybrid.cpulist(text)


if __name__ == '__main__':
    unittest.main()
