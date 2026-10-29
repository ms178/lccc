import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('cpu_evidence', Path(__file__).resolve().parents[2] / 'scripts/publish_cpu_runtime_evidence.py')
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
    def test_symbol_excerpt_keeps_clones_but_not_other_functions(self):
        disassembly = '''Disassembly of section .text:
0000000000000000 <target.constprop.0>:
  0: add %eax,%eax
0000000000000002 <unrelated>:
  2: ret
0000000000000003 <target>:
  3: ret
Disassembly of section .fini:
  trailing
'''
        excerpt = m.symbol_excerpt(disassembly, ('target',))
        self.assertIn('<target.constprop.0>', excerpt)
        self.assertIn('<target>', excerpt)
        self.assertNotIn('unrelated', excerpt)
        self.assertNotIn('trailing', excerpt)
        self.assertIn('may have been inlined', m.symbol_excerpt(disassembly, ('missing',)))
        self.assertIn('excerpt truncated', m.symbol_excerpt(disassembly, ('target',), 10))

    def test_identity_control_is_not_hidden_by_timing_ratio(self):
        r = m.summaries(report(), 'portable')[0]
        self.assertTrue(r['whole_executable_identical'])
        self.assertEqual(r['ratios_after_over']['before']['median'], 0.99)

    def test_vendor_ratio_is_reported_when_requested(self):
        r = report()
        r['compilers'].append({'key': 'icx', 'binary_sha256': 'vendor'})
        b = r['benchmarks'][0]
        b['correctness']['icx'] = copy.deepcopy(b['correctness']['gcc'])
        b['runtime']['icx'] = copy.deepcopy(b['runtime']['gcc'])
        b['compile']['icx'] = {'binary_sha256': 'vendor-output'}
        b['ratios_from_lccc']['icx'] = {'median': 1.2, 'median_ci95_bootstrap': [1.1, 1.3]}
        row = m.summaries(r, 'portable')[0]
        self.assertEqual(row['ratios_after_over']['icx']['median'], 1.2)
        b['correctness']['icx']['matches_baseline'] = False
        with self.assertRaises(ValueError):
            m.summaries(r, 'portable')

    def test_rolled_control_must_be_correct_and_is_reported(self):
        r = report()
        r['compilers'].append({'key': 'rolled', 'binary_sha256': 'wrapper'})
        b = r['benchmarks'][0]
        b['correctness']['rolled'] = copy.deepcopy(b['correctness']['lccc'])
        b['runtime']['rolled'] = copy.deepcopy(b['runtime']['lccc'])
        b['compile']['rolled'] = {'binary_sha256': 'same'}
        b['ratios_from_lccc']['rolled'] = {'median': 0.8, 'median_ci95_bootstrap': [0.7, 0.9]}
        row = m.summaries(r, 'portable')[0]
        self.assertTrue(row['whole_executable_identical_rolled'])
        self.assertEqual(row['ratios_after_over']['rolled']['median'], 0.8)
        b['correctness']['rolled']['matches_baseline'] = False
        with self.assertRaises(ValueError):
            m.summaries(r, 'portable')

    def test_default_evidence_fits_the_whole_job_annotation_budget(self):
        groups = [
            ['zlib_ng_slide_hash'],
            ['fp64_product_map', 'fp32_product_map'],
            ['linux_list_sort', 'sqlite_vdbe_serial'],
            ['byte_load_reassembly', 'zlib_ng_compare256', 'arith_loop', 'nbody'],
        ]
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for profile in ('portable', 'native'):
                for name in [case for group in groups for case in group]:
                    for arm in ('lccc', 'gcc', 'icx', 'rolled', 'before', 'clang'):
                        path = root / f'{profile}-artifacts' / name / arm / 'assembly.txt'
                        path.parent.mkdir(parents=True, exist_ok=True)
                        symbols = m.PRIORITY_SYMBOLS.get(name, ('main',))
                        path.write_text(''.join(
                            f'0000000000000000 <{symbol}>:\n' + '  0: sqrtsd %xmm0,%xmm0\n' * 300
                            for symbol in symbols))
            total = 0
            for profile in ('portable', 'native'):
                for group in groups:
                    with self.subTest(profile=profile, group=group), mock.patch.object(m, 'annotate') as annotate:
                        m.publish_disassembly(root, profile, group)
                        self.assertEqual(annotate.call_count, len(group))
                        total += annotate.call_count
                        for call in annotate.call_args_list:
                            self.assertLess(len(call.args[1].encode()), 32000)
                            self.assertIn('=== lccc', call.args[1])
                            self.assertIn('=== gcc', call.args[1])
                            if 'product_map' in call.args[0]:
                                self.assertIn('=== lccc/main ===', call.args[1])
                                self.assertIn('=== gcc/main ===', call.args[1])
                                self.assertNotIn('=== rolled/main ===', call.args[1])
            self.assertEqual(total, 18)
            # AVX, paired timing, the ablation, batched timing and Callgrind
            # summaries must remain visible alongside all assembly.
            self.assertLessEqual(6 + 8 + 1 + 6 + 1 + total, 50)

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
