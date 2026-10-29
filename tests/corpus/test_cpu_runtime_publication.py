import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('cpu_evidence', Path(__file__).resolve().parents[2] / 'scripts/publish_cpu_runtime_evidence.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def report():
    arms = ('lccc', 'before', 'gcc', 'clang')
    return {'complete': True, 'settings': {'opt': '-O2', 'common_cflags': ['-std=gnu17'], 'reps': 31, 'warmup': 3},
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

    def test_focused_windows_find_backedges_and_call_sites(self):
        prefix = ''.join(f'  {n:04x}: 90                 nop\n' for n in range(256, 288))
        disassembly = (
            '0000000000000100 <main>:\n' + prefix +
            '  0120: 8b 00              mov (%rax),%eax\n'
            '  0122: 75 fc              jne 120 <main+0x20>\n'
            '  0124: e8 00              call 200 <fp_product_map.constprop.0>\n'
            '  0129: c3                 ret\n'
            '0000000000000200 <other>:\n  0200: c3 ret\n'
        )
        loop = m.focused_excerpt(disassembly, 'main', 1600, 'last-loop')
        self.assertIn('jne 120 <main+0x20>', loop)
        self.assertIn('mov (%rax),%eax', loop)
        self.assertNotIn('<other>', loop)
        caller = m.focused_excerpt(disassembly, 'main', 1600, 'call')
        self.assertIn('call 200 <fp_product_map.constprop.0>', caller)
        self.assertNotIn('<other>', caller)
        self.assertIn('focused site not found', m.focused_excerpt(disassembly, 'other', 1000, 'call'))
        decoder = ('0000000000000300 <vdbe_serial_get>:\n' + prefix +
                   '  0320: 0f b7 07           movzwl (%rdi),%eax\n'
                   '  0323: 66 c1 c0 08        rol $0x8,%ax\n'
                   '  0327: 48 0f bf c0        movswq %ax,%rax\n'
                   '  032b: c3                 ret\n')
        window = m.focused_excerpt(decoder, 'vdbe_serial_get', 1000, 'narrow-be')
        self.assertIn('rol $0x8,%ax', window)
        self.assertIn('movswq %ax,%rax', window)
        loops = ('0000000000000100 <search>:\n'
                 '  0100: 90                 nop\n'
                 '  0120: 75 de              jne 100 <search>\n'
                 '  0124: 8a 00              mov (%rax),%al\n'
                 '  0126: 75 fc              jne 124 <search+0x24>\n'
                 '  0128: c3                 ret\n')
        shortest = m.focused_excerpt(loops, 'search', 1000, 'shortest-loop')
        self.assertIn('[shortest backward branch; not a measured hotness claim]', shortest)
        self.assertIn('jne 124 <search+0x24>', shortest)

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
            ['linux_list_sort', 'sqlite_vdbe_serial', 'glibc_strstr'],
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
                        lines = 800 if name == 'byte_load_reassembly' else 300
                        path.write_text(''.join(
                            f'0000000000000000 <{symbol}>:\n' + '  0: sqrtsd %xmm0,%xmm0\n' * lines
                            for symbol in symbols))
            total = 0
            for profile in ('portable', 'native'):
                for group in groups:
                    with self.subTest(profile=profile, group=group), mock.patch.object(m, 'annotate') as annotate:
                        m.publish_disassembly(root, profile, group)
                        extra = {'zlib_ng_slide_hash', 'fp64_product_map', 'fp32_product_map'}
                        self.assertEqual(annotate.call_count, len(group) + len(extra.intersection(group)))
                        total += annotate.call_count
                        for call in annotate.call_args_list:
                            # GitHub truncates a single annotation at 4096 bytes.
                            self.assertLessEqual(len(call.args[1].encode()), 3900)
                            self.assertIn('=== lccc', call.args[1])
                            self.assertIn('=== gcc', call.args[1])
                            if 'product_map' in call.args[0] and call.args[0].endswith('/caller'):
                                self.assertIn('=== lccc/main ===', call.args[1])
                                self.assertIn('=== before/main ===', call.args[1])
                                self.assertIn('=== gcc/main ===', call.args[1])
                                self.assertNotIn('=== rolled/main ===', call.args[1])
                            if call.args[0].endswith('/sqlite_vdbe_serial/narrow-be'):
                                self.assertIn('=== before/vdbe_serial_get ===', call.args[1])
            self.assertEqual(total, 26)
            # AVX, paired timing for up to 20 cases (including identities),
            # batched timing, the join A/B, the zero-extension ablation and the
            # Callgrind summaries must remain visible alongside every excerpt.
            # Rows are packed, so count the real packing rather than a grid.
            rows = [{'profile': 'native', 'name': f'case{i}', 'source_sha256': 'a' * 64,
                     'whole_executable_identical': False, 'whole_executable_identical_rolled': False,
                     'ratios_after_over': {arm: {'median': 1.0123, 'ci95': [0.9912, 1.0345]}
                                           for arm in ('before', 'gcc', 'clang', 'icx', 'rolled')}}
                    for i in range(20)]
            packed = sum(len(m.row_notices(rows)) for _ in ('portable', 'native'))
            self.assertLessEqual(packed, 6, '20 paired rows must pack into three notices per profile')
            self.assertLessEqual(6 + packed + 6 + 1 + 1 + 1 + total, 50)

    def test_native_profile_requires_actual_native_flag(self):
        r = report()
        with self.assertRaises(ValueError): m.summaries(r, 'native')
        r['settings']['common_cflags'].append('-march=native')
        self.assertEqual(len(m.summaries(r, 'native')), 1)
        with self.assertRaises(ValueError): m.summaries(r, 'portable')

    def test_optimization_controls_keep_partial_results_distinct_from_complete_evidence(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            portable = root / 'portable-strided-ab'
            native = root / 'native-strided-ab'
            cg = root / 'callgrind-strided-ab'
            for directory in (portable, native, cg):
                directory.mkdir()
            comparison = {'median': 1.1, 'exploratory_ci95': [1.0, 1.2],
                          'identical_binary_to_reference': False, 'minimum_sample_seconds': .2}
            data = {'complete': True, 'reference': 'disabled', 'rounds': 9,
                    'pinning': {'applied': True},
                    'rows': [{'name': 'byte_load_reassembly', 'samples': [{}] * 9,
                              'comparisons': {'lccc': comparison, 'gcc': comparison}}]}
            (portable / 'results.json').write_text(json.dumps(data))
            (native / 'results.json').write_text(json.dumps({'complete': False}))
            (cg / 'index.json').write_text(json.dumps({
                'complete': True, 'failures': [], 'results': {
                    'byte_load_reassembly': {'metrics': {'ir_ratio': 0.75}}}}))
            summary = m.optimization_controls(root, {'byte_load_reassembly'})
            self.assertEqual(set(summary), {'strided'})
            result = summary['strided']
            self.assertTrue(result['profiles']['portable']['complete'])
            self.assertEqual(result['profiles']['portable']['ratios']['lccc']['median'], 1.1)
            self.assertEqual(len(result['profiles']['portable']['sha256']), 64)
            self.assertFalse(result['profiles']['native']['complete'])
            self.assertNotIn('ratios', result['profiles']['native'])
            self.assertEqual(result['callgrind']['ir_ratio'], 0.75)
            (cg / 'index.json').write_text(json.dumps({'complete': True, 'failures': [{'name': 'x'}]}))
            self.assertFalse(m.optimization_controls(root, {'byte_load_reassembly'})['strided']['callgrind']['complete'])
            self.assertEqual(m.optimization_controls(root, set()), {})

    def test_runtime_rows_pack_greedily_within_the_annotation_limit(self):
        rows = [{'name': f'case{i}', 'x': 'y' * 100} for i in range(20)]
        notices = m.row_notices(rows)
        self.assertTrue(notices, 'rows must be published')
        self.assertLess(len(notices), 5, 'packing must beat a fixed five-row grid')
        joined = []
        for notice in notices:
            self.assertLessEqual(len(notice.encode()), 3900)
            joined.extend(json.loads(notice))
        self.assertEqual(joined, rows, 'packing must neither drop nor duplicate rows')
        with self.assertRaises(ValueError):
            m.row_notices([{'name': 'huge', 'x': 'y' * 4000}])
        self.assertEqual(m.row_notices([]), [])

    def test_control_digest_keeps_shape_and_marks_missing_evidence(self):
        controls = {'strided': {
            'case': 'byte_load_reassembly',
            'profiles': {'portable': {'available': False},
                         'native': {'complete': True, 'rounds': 9, 'ratios': {
                             'lccc': {'median': 1.123456, 'identical': False},
                             'gcc': {'median': 1.5, 'identical': False}}}},
            'callgrind': {'complete': True, 'ir_ratio': 0.87654321}}}
        digest = m.control_digest(controls)['strided']
        self.assertEqual(digest['case'], 'byte_load_reassembly')
        self.assertEqual(digest['profiles']['portable'], {'available': False})
        self.assertEqual(digest['profiles']['native']['ratios']['lccc'], [1.1235, False])
        self.assertEqual(digest['callgrind_ir'], 0.87654)
        controls['strided']['callgrind'] = {'complete': False}
        self.assertIsNone(m.control_digest(controls)['strided']['callgrind_ir'])

    def test_rejection_details_cover_independent_failures_without_extra_notices(self):
        r = report()
        b = r['benchmarks'][0]
        b['baseline_compiler'] = 'gcc'
        b['compile']['clang'].update(ok=False, error='compiler error\nsecond line')
        b['correctness']['lccc'].update(matches_baseline=False, output='wrong % result\n')
        b['correctness']['gcc']['output'] = 'oracle result\n'
        b['correctness']['before']['all_runs_successful'] = False
        b['runs'] = {'before': [{'ok': False, 'returncode': 139, 'stderr': 'signal'}]}
        details = json.loads(m.rejection_details(r))
        self.assertEqual(details['omitted'], 0)
        text = ' '.join(details['details'])
        self.assertIn('fixture/clang compile: compiler error second line', text)
        self.assertIn('fixture/lccc output mismatch', text)
        self.assertIn('wrong % result', text)
        self.assertIn('oracle result', text)
        self.assertIn('fixture/before execution: signal', text)
        with self.assertRaises(ValueError):
            m.summaries(r, 'portable', {'fixture'})

    def test_rejection_details_are_bounded_even_for_large_non_ascii_errors(self):
        r = {'unavailable_compilers': {f'compiler{i}': '\u2603\\"%' * 2000 for i in range(30)}}
        text = m.rejection_details(r)
        details = json.loads(text)
        self.assertEqual(len(details['details']), 8)
        self.assertEqual(details['omitted'], 22)
        self.assertLessEqual(len(text.encode()), 3000)
        self.assertNotIn('\n', text)

    def test_selection_must_match_when_requested(self):
        r = report()
        self.assertEqual(len(m.summaries(r, 'portable', {'fixture'})), 1)
        with self.assertRaises(ValueError):
            m.summaries(r, 'portable', {'fixture', 'missing'})
        with self.assertRaises(ValueError):
            m.summaries(r, 'portable', {'other'})

    def test_compiler_set_and_unique_identities_must_match_when_requested(self):
        r = report()
        expected = {'lccc', 'before', 'gcc', 'clang'}
        self.assertEqual(len(m.summaries(r, 'portable', {'fixture'}, expected)), 1)
        with self.assertRaises(ValueError):
            m.summaries(r, 'portable', {'fixture'}, expected | {'icx'})
        r['compilers'].append(copy.deepcopy(r['compilers'][0]))
        with self.assertRaises(ValueError):
            m.summaries(r, 'portable', {'fixture'}, expected)

    def test_cli_keeps_the_strict_failure_and_reports_both_profiles(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            r = report()
            r['benchmarks'][0]['correctness']['before']['matches_baseline'] = False
            (root / 'portable.json').write_text(json.dumps(r))
            r['settings']['common_cflags'].append('-march=native')
            (root / 'native.json').write_text(json.dumps(r))
            with mock.patch.object(m.sys, 'argv', ['publisher', str(root), '--expected-cases', 'fixture']), \
                 mock.patch.object(m, 'annotate') as annotate:
                self.assertEqual(m.main(), 1)
            self.assertEqual(annotate.call_count, 2)
            for call in annotate.call_args_list:
                self.assertEqual(call.args[0], 'CPU runtime report rejected')
                self.assertIn('fixture/before output mismatch', call.args[1])
                self.assertLessEqual(len(call.args[1].encode()), 3900)

    def test_refuse_unsound_or_incomplete_evidence(self):
        r = report()
        variants = []
        v = copy.deepcopy(r); v['complete'] = False; variants.append(v)
        v = copy.deepcopy(r); v['unavailable_compilers'] = {'icx': 'not found'}; variants.append(v)
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
