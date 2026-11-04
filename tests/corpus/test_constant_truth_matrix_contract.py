"""Pinned broken references never relax the candidate's semantic contract."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    'constant_truth_matrix', ROOT / 'tests/regression/check_constant_truth_diagnostics.py')
MATRIX = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MATRIX)
CONTROL = '(void)__builtin_constant_p(({ return 99; 0; }));'
CASE = 'constant_p_stmt_expr_is_unevaluated'


def result(code=0, stderr='', stdout=''):
    return SimpleNamespace(returncode=code, stderr=stderr, stdout=stdout)


class MatrixContract(unittest.TestCase):
    def test_candidate_missing_warning_cannot_use_reference_exception(self):
        row = MATRIX.diagnostic_outcome(result(), 'lccc', CASE, 1)
        self.assertFalse(row['passed'])
        self.assertFalse(row['semantic_passed'])

    def test_reference_observation_is_not_semantic_success(self):
        row = MATRIX.diagnostic_outcome(result(), 'icx', CASE, 1)
        self.assertTrue(row['passed'])
        self.assertFalse(row['semantic_passed'])

    def test_corrected_reference_diagnostic_is_flagged_for_profile_review(self):
        row = MATRIX.diagnostic_outcome(result(stderr='warning: return [-Wreturn-type]'),
                                        'icx', CASE, 1)
        self.assertFalse(row['passed'])
        self.assertTrue(row['semantic_passed'])

    def test_accidental_candidate_profile_cannot_relax_any_result(self):
        with mock.patch.dict(MATRIX.REFERENCE_RETURN_COUNTS, {('lccc', CASE): 0}):
            self.assertFalse(MATRIX.diagnostic_outcome(result(), 'lccc', CASE, 1)['passed'])
        with mock.patch.dict(MATRIX.REFERENCE_RUNTIME_EXIT_CODES, {('lccc', CONTROL): 1}):
            self.assertFalse(MATRIX.runtime_outcome(result(), result(1), 'lccc', CONTROL)['passed'])

    def test_extra_unclassified_or_duplicate_warnings_fail(self):
        for stderr in ['warning: unknown', 'warning: return [-Wreturn-type]\nwarning: unknown',
                       'warning: return [-Wreturn-type]\nwarning: return [-Wreturn-type]']:
            self.assertFalse(MATRIX.diagnostic_outcome(result(stderr=stderr), 'lccc', CASE, 1)['passed'])

    def test_compile_failure_cannot_pass_either_contract(self):
        self.assertFalse(MATRIX.diagnostic_outcome(result(1), 'clang', CASE, 1)['passed'])
        self.assertFalse(MATRIX.runtime_outcome(result(1), result(1), 'icx', CONTROL)['passed'])
        self.assertFalse(MATRIX.runtime_outcome(result(1), None, 'icx', CONTROL)['passed'])

    def test_exact_auxiliary_warning_is_required(self):
        stderr = 'warning: unreachable [-Wswitch-unreachable]\nwarning: return [-Wreturn-type]'
        row = MATRIX.diagnostic_outcome(result(stderr=stderr), 'gcc', 'case_entry_after_return', 1)
        self.assertTrue(row['passed'])
        self.assertTrue(row['semantic_passed'])

    def test_reference_runtime_failure_is_explicit_but_never_a_candidate_waiver(self):
        for family in ['clang', 'icx']:
            row = MATRIX.runtime_outcome(result(), result(1), family, CONTROL)
            self.assertTrue(row['passed'])
            self.assertFalse(row['semantic_passed'])
        row = MATRIX.runtime_outcome(result(), result(1), 'lccc', CONTROL)
        self.assertFalse(row['passed'])
        self.assertFalse(row['semantic_passed'])

    def test_unexpected_native_status_or_output_fails_even_known_reference(self):
        for executed in [result(2), result(-11), result(1, stdout='unexpected'),
                         result(1, stderr='unexpected')]:
            self.assertFalse(MATRIX.runtime_outcome(result(), executed, 'icx', CONTROL)['passed'])
        self.assertFalse(MATRIX.runtime_outcome(result(), result(1), 'icx', 'unlisted')['passed'])

    def test_candidate_normal_runtime_requires_zero_and_empty_output(self):
        row = MATRIX.runtime_outcome(result(), result(), 'lccc', CONTROL)
        self.assertTrue(row['passed'])
        self.assertTrue(row['semantic_passed'])
