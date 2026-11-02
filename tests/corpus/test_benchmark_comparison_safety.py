"""A fast or unstable wrong answer must never become a compiler speed win."""
from itertools import product
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

    def test_candidate_warmup_output_cannot_be_discarded(self):
        row = entry({'gcc': [run()], 'lccc': [run()]})
        row['warmups'] = {'gcc': [run()], 'lccc': [run('wrong\n')]}
        bench.evaluate_correctness(row, ['gcc', 'lccc'])
        self.assertIsNot(row['correctness']['lccc']['matches_baseline'], True)

    def test_control_warmup_instability_cannot_certify_a_candidate(self):
        row = entry({'gcc': [run()], 'lccc': [run()]})
        row['warmups'] = {'gcc': [run('wrong\n')], 'lccc': [run()]}
        bench.evaluate_correctness(row, ['gcc', 'lccc'])
        self.assertIsNot(row['correctness']['lccc']['matches_baseline'], True)

    def test_failed_warmup_cannot_certify_retained_successes(self):
        for arm in ['gcc', 'lccc']:
            with self.subTest(arm=arm):
                row = entry({'gcc': [run()], 'lccc': [run()]})
                row['warmups'] = {arm: [run(ok=False)]}
                bench.evaluate_correctness(row, ['gcc', 'lccc'])
                self.assertIsNot(row['correctness']['lccc']['matches_baseline'], True)

    def test_all_bounded_warmup_and_sample_states_fail_closed(self):
        states = [(), (run(),), (run('wrong\n'),), (run(ok=False),),
                  (run(), run('wrong\n')), (run(), run(ok=False))]

        def valid(timed, warmups):
            executions = [*warmups, *timed]
            return bool(timed) and all(item['ok'] for item in executions) and len(
                {item['stdout'] for item in executions}) == 1

        for gcc_warmups, gcc_timed, candidate_warmups, candidate_timed in product(states, repeat=4):
            row = entry({'gcc': list(gcc_timed), 'lccc': list(candidate_timed)})
            row['warmups'] = {'gcc': list(gcc_warmups), 'lccc': list(candidate_warmups)}
            bench.evaluate_correctness(row, ['gcc', 'lccc'])
            expected = (valid(gcc_timed, gcc_warmups) and valid(candidate_timed, candidate_warmups)
                        and gcc_timed[0]['stdout'] == candidate_timed[0]['stdout'])
            with self.subTest(warmups=row['warmups'], timed=row['runs']):
                self.assertEqual(row['correctness']['lccc']['matches_baseline'] is True, expected)

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
        row['warmups'] = {'gcc': [run()], 'lccc': [run()]}
        bench.evaluate_correctness(row, ['gcc', 'lccc'])
        self.assertEqual(len(row['runs']['lccc']), 2)
        for value in row['correctness'].values():
            self.assertTrue(value['matches_baseline'])
            self.assertTrue(value['stable_within_compiler'])

    def test_an_explicitly_gcc_free_experiment_can_use_a_reference_control(self):
        row = entry({'clang': [run()], 'lccc': [run()]})
        bench.evaluate_correctness(row, ['clang', 'lccc'])
        self.assertTrue(row['correctness']['lccc']['matches_baseline'])


if __name__ == '__main__':
    unittest.main()
