import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('cpu_evidence', Path(__file__).with_name('publish_cpu_runtime_evidence.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def report():
    arms = ('lccc', 'before', 'gcc', 'clang')
    return {'settings': {'opt': '-O2', 'common_cflags': ['-std=gnu17'], 'reps': 31, 'warmup': 3},
            'metadata': {'pinning': {'applied': True}},
            'compilers': [{'key': a, 'binary_sha256': a} for a in arms],
            'benchmarks': [{'name': 'fixture', 'source_sha256': 'source',
                            'correctness': {a: dict.fromkeys(('all_runs_successful', 'matches_baseline', 'stable_within_compiler'), True) for a in arms},
                            'runtime': {a: {'wall': {'n': 31}} for a in arms},
                            'compile': {a: {'binary_sha256': 'same' if a in ('lccc', 'before') else a} for a in arms},
                            'ratios_from_lccc': {a: {'median': 0.99, 'median_ci95_bootstrap': [0.9, 1.1]} for a in arms if a != 'lccc'}}]}


class PublicationTests(unittest.TestCase):
    def test_identity_control_is_not_hidden_by_timing_ratio(self):
        r = m.summaries(report(), 'portable')[0]
        self.assertTrue(r['whole_executable_identical'])
        self.assertEqual(r['ratios_after_over']['before']['median'], 0.99)

    def test_native_profile_requires_actual_native_flag(self):
        r = report()
        with self.assertRaises(ValueError): m.summaries(r, 'native')
        r['settings']['common_cflags'].append('-march=native')
        self.assertEqual(len(m.summaries(r, 'native')), 1)
        with self.assertRaises(ValueError): m.summaries(r, 'portable')

    def test_refuse_unsound_or_incomplete_evidence(self):
        r = report()
        variants = []
        for key, value in [('reps', 3), ('warmup', 1), ('opt', '-O3')]:
            v = copy.deepcopy(r); v['settings'][key] = value; variants.append(v)
        v = copy.deepcopy(r); v['metadata']['pinning']['applied'] = False; variants.append(v)
        v = copy.deepcopy(r); v['benchmarks'][0]['correctness']['before']['matches_baseline'] = False; variants.append(v)
        v = copy.deepcopy(r); v['benchmarks'][0]['runtime']['before']['wall']['n'] = 30; variants.append(v)
        v = copy.deepcopy(r); v['compilers'].pop(); variants.append(v)
        v = copy.deepcopy(r); v['benchmarks'] *= 2; variants.append(v)
        for v in variants:
            with self.subTest(v=v), self.assertRaises(ValueError): m.summaries(v, 'portable')


if __name__ == '__main__': unittest.main()
