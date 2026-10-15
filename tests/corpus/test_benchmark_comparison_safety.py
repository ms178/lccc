"""A fast or unstable wrong answer must never become a compiler speed win."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tests.benchmark import run_benchmarks as bench


def run(stdout='correct\n', ok=True):
    return {'ok': ok, 'stdout': stdout, 'wall_s': .1}


def entry(runs):
    return {'name': 'fixture', 'runs': runs, 'correctness': {}, 'rounds': [],
            'ratios_to_gcc': {}, 'ratios_from_lccc': {}, 'runtime': {}}


class ComparisonSafety(unittest.TestCase):
    def test_unstable_gcc_cannot_certify_a_matching_first_output(self):
        row = entry({'gcc': [run(), run('wrong\n')], 'lccc': [run(), run()]})
        bench.evaluate_correctness(row, ['gcc', 'lccc'])
        self.assertIsNot(row['correctness']['lccc']['matches_baseline'], True)

    def test_missing_gcc_never_promotes_the_candidate_to_its_own_oracle(self):
        row = entry({'gcc': [run(ok=False)], 'lccc': [run()]})
        bench.evaluate_correctness(row, ['gcc', 'lccc'])
        self.assertIsNot(row['correctness']['lccc']['matches_baseline'], True)

    def test_failed_sample_cannot_be_hidden_by_successful_samples(self):
        row = entry({'gcc': [run(), run()], 'lccc': [run(), run(ok=False)]})
        bench.evaluate_correctness(row, ['gcc', 'lccc'])
        self.assertIsNot(row['correctness']['lccc']['matches_baseline'], True)

    def test_gcc_aggregate_requires_a_correct_reference(self):
        row = entry({})
        row['correctness'] = {'lccc': {'matches_baseline': True}, 'gcc': {'matches_baseline': False}}
        row['ratios_to_gcc']['lccc'] = {'median': .5}
        self.assertIsNone(bench.aggregate_lccc_vs_gcc([row]))

    def test_best_reference_is_selected_only_from_correct_compilers(self):
        row = entry({})
        row['correctness'] = {key: {'matches_baseline': key != 'bad'} for key in ['lccc', 'gcc', 'bad']}
        row['runtime'] = {key: {'wall': {'median': value}} for key, value in [('gcc', 1), ('bad', .01)]}
        self.assertEqual(bench.best_reference_for_entry(row), ('gcc', 1))

    def test_stable_complete_arms_remain_comparable(self):
        row = entry({'gcc': [run(), run()], 'lccc': [run(), run()]})
        bench.evaluate_correctness(row, ['gcc', 'lccc'])
        for value in row['correctness'].values():
            self.assertTrue(value['matches_baseline'])
            self.assertTrue(value['stable_within_compiler'])

    def test_an_explicitly_gcc_free_experiment_can_use_a_reference_control(self):
        row = entry({'clang': [run()], 'lccc': [run()]})
        bench.evaluate_correctness(row, ['clang', 'lccc'])
        self.assertTrue(row['correctness']['lccc']['matches_baseline'])


if __name__ == '__main__':
    unittest.main()
