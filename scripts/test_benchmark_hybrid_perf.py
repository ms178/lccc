#!/usr/bin/env python3
import importlib.util
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
                    '--reps', '1', '--warmup', '0', '--perf-reps', '1',
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
            self.assertEqual(report.count('"compiler":'), 2)

    def test_cpulist_rejects_malformed(self):
        for text in ('', '1-0', 'x', '1--2'):
            with self.assertRaises(ValueError):
                hybrid.cpulist(text)


if __name__ == '__main__':
    unittest.main()
