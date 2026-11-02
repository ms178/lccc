"""Raw paired executions, not cached summaries, authorize performance claims."""
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/corpus'))
from test_compiler_comparison_reports import summary


def fixture():
    arms = ['lccc', 'gcc', 'prior']
    rows = []
    for case in summary.bench.BENCHMARKS:
        runs = {
            arm: [{'ok': True, 'stdout': 'correct\n',
                   'wall_s': .2 if arm == 'lccc' else .1} for _ in range(9)]
            for arm in arms
        }
        rows.append({
            'name': case.name,
            'correctness': {
                arm: {'matches_baseline': True, 'stable_within_compiler': True}
                for arm in arms
            },
            'runs': runs,
            'rounds': [
                {'round': index, 'order': arms[:],
                 'runs': {arm: runs[arm][index] for arm in arms}}
                for index in range(9)
            ],
            'runtime': {
                arm: {'wall': {'median': .2 if arm == 'lccc' else .1}}
                for arm in arms
            },
            'ratios_from_lccc': {
                arm: {'n': 9, 'median': 2.0} for arm in ['gcc', 'prior']
            },
        })
    return {
        'complete': True,
        'benchmarks': rows,
        'compilers': [{'key': arm} for arm in arms],
        'settings': {'skip_perf': False},
    }


def first_comparison(data):
    return summary.summarize(data)['rows'][0]['ratios']['prior']


class TimingProvenance(unittest.TestCase):
    def test_valid_raw_pairs_still_admitted(self):
        self.assertTrue(first_comparison(fixture())['eligible'])

    def test_cached_ratio_cannot_invent_speedup(self):
        data = fixture()
        data['benchmarks'][0]['ratios_from_lccc']['prior']['median'] = .01
        result = first_comparison(data)
        self.assertFalse(result['eligible'])
        self.assertEqual(result['paired']['median'], 2.0)

    def test_cached_floor_cannot_admit_short_raw_samples(self):
        data = fixture()
        for run in data['benchmarks'][0]['runs']['lccc']:
            run['wall_s'] = .001
        result = first_comparison(data)
        self.assertFalse(result['eligible'])
        self.assertFalse(result['above_floor'])

    def test_missing_duplicate_or_mismatched_pairs_are_not_evidence(self):
        for mode in ['missing', 'duplicate', 'mismatch', 'reorder', 'fake_order']:
            with self.subTest(mode=mode):
                data = fixture()
                row = data['benchmarks'][0]
                if mode == 'missing':
                    row['rounds'] = []
                elif mode == 'duplicate':
                    row['rounds'][1]['round'] = 0
                elif mode == 'mismatch':
                    row['rounds'][0]['runs']['prior'] = {
                        **row['runs']['prior'][0], 'wall_s': 5,
                    }
                elif mode == 'reorder':
                    row['rounds'].reverse()
                else:
                    row['rounds'][0]['order'] = ['lccc', 'gcc', 'gcc']
                self.assertFalse(first_comparison(data)['eligible'])

    def test_invalid_raw_timing_never_enters_aggregate(self):
        for value in [0, -1, float('nan'), float('inf'), None, True, '0.1']:
            with self.subTest(value=value):
                data = fixture()
                data['benchmarks'][0]['runs']['lccc'][0]['wall_s'] = value
                self.assertFalse(first_comparison(data)['eligible'])

    def test_incomplete_reports_cannot_be_certified(self):
        data = fixture()
        data['complete'] = False
        with self.assertRaises(ValueError):
            summary.summarize(data)

    def test_forged_uncertainty_is_recomputed_not_copied(self):
        data = fixture()
        data['benchmarks'][0]['ratios_from_lccc']['prior'].update(
            median_ci95_bootstrap=[.01, .02], raw=[{'round': 0, 'ratio': .01}])
        result = first_comparison(data)
        self.assertTrue(result['eligible'])
        self.assertEqual(result['paired']['median_ci95_bootstrap'], [2.0, 2.0])
        self.assertEqual(len(result['paired']['raw']), 9)
        self.assertTrue(all(pair['ratio'] == 2.0 for pair in result['paired']['raw']))

    def test_sample_counts_are_integral_and_match_the_observed_pairs(self):
        for value in [True, 9.0, '9', 0, -1, 1000]:
            with self.subTest(value=value):
                data = fixture()
                data['benchmarks'][0]['ratios_from_lccc']['prior']['n'] = value
                self.assertFalse(first_comparison(data)['eligible'])

    def test_boolean_round_ids_and_statuses_are_not_numeric_evidence(self):
        for where in ['round', 'status']:
            with self.subTest(where=where):
                data = fixture()
                row = data['benchmarks'][0]
                if where == 'round':
                    row['rounds'][0]['round'] = False
                else:
                    row['runs']['lccc'][0]['ok'] = 1
                self.assertFalse(first_comparison(data)['eligible'])


if __name__ == '__main__':
    unittest.main()
