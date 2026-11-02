#!/usr/bin/env python3
"""Survey contracts are benchmark ids plus flags, not just source filenames."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('benchmark_codegen_survey', ROOT / 'scripts/benchmark_codegen_survey.py')
survey = importlib.util.module_from_spec(spec)
spec.loader.exec_module(survey)


class BenchmarkSurveyTests(unittest.TestCase):
    def test_scaled_alias_has_distinct_workload_flags(self):
        normal, scaled = [survey.case_contract(c, '-O2 -fwrapv') for c in
                          survey.select_cases('linux_find_bit,linux_find_bit_scaled')]
        self.assertEqual(normal['source'], scaled['source'])
        self.assertNotIn('-DPASSES=', normal['flags'])
        self.assertIn('-DPASSES=65536', scaled['flags'])

    def test_match_rich_lz4_preserves_both_defines(self):
        case = survey.case_contract(survey.select_cases('lz4_match_extend')[0], '-O2')
        self.assertEqual(Path(case['source']).name, 'lz4_compress.c')
        self.assertIn('-DMATCH_RICH=1', case['flags'])
        self.assertIn('-DPASSES=2048', case['flags'])

    def test_selection_is_explicit_unique_and_known(self):
        for names in ['', 'unknown', 'sieve,sieve', 'sieve,']:
            with self.subTest(names=names), self.assertRaises(ValueError):
                survey.select_cases(names)

    def test_missing_duplicate_failed_or_wrong_flag_arms_are_not_coverage(self):
        keys = ['lccc', 'gcc', 'clang', 'icx']
        row = dict(source='fixture.c', errors=[], records=[
            dict(key=key, flags='-O2 -DMATCH_RICH=1') for key in keys])
        self.assertIs(survey.validate_result([row], 'fixture.c', keys, '-O2 -DMATCH_RICH=1'), row)
        mutants = [[], [None], [dict(row, errors=['failure'])],
                   [dict(row, records=row['records'][:-1])],
                   [dict(row, records=[row['records'][0]] * 4)],
                   [dict(row, records=[dict(r, flags='-O2') for r in row['records']])]]
        for data in mutants:
            with self.subTest(data=data), self.assertRaises(ValueError):
                survey.validate_result(data, 'fixture.c', keys, '-O2 -DMATCH_RICH=1')


if __name__ == '__main__':
    unittest.main()
