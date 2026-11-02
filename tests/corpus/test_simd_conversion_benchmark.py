"""Correctness policy for the matched-ISA SIMD timing harness."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    'simd_conversion_benchmark', ROOT / 'scripts/benchmark_simd_conversion.py'
)
BENCHMARK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BENCHMARK)


class CandidateCorrectnessPolicy(unittest.TestCase):
    def row(self, before='old', after='good', gcc='good', clang='good'):
        return {
            'name': 'zero_extend',
            'artifacts': {
                name: {'stdout': output}
                for name, output in (
                    ('before', before), ('after', after), ('gcc', gcc), ('clang', clang)
                )
            },
        }

    def test_incorrect_historical_before_is_recorded_but_not_timed(self):
        row = self.row()
        valid = {name: Path(name) for name in ('before', 'after', 'gcc', 'clang')}

        BENCHMARK.retain_correct_candidates(row, valid, 'good')

        self.assertNotIn('before', valid)
        self.assertEqual(row['artifacts']['before']['output_matches_gcc'], False)
        self.assertEqual(row['artifacts']['before']['status'], 'correctness-mismatch')
        self.assertEqual(set(valid), {'after', 'gcc', 'clang'})

    def test_correct_historical_before_remains_a_comparable_candidate(self):
        row = self.row(before='good')
        valid = {name: Path(name) for name in ('before', 'after', 'gcc', 'clang')}

        BENCHMARK.retain_correct_candidates(row, valid, 'good')

        self.assertIn('before', valid)
        self.assertEqual(row['artifacts']['before']['status'], 'correct')

    def test_current_oracle_mismatch_aborts_instead_of_being_timed(self):
        row = self.row(after='wrong')
        valid = {name: Path(name) for name in ('before', 'after', 'gcc', 'clang')}

        with self.assertRaisesRegex(RuntimeError, 'zero_extend/after'):
            BENCHMARK.retain_correct_candidates(row, valid, 'good')
        self.assertEqual(row['artifacts']['after']['status'], 'correctness-mismatch')


if __name__ == '__main__':
    unittest.main()
