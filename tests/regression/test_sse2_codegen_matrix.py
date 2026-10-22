#!/usr/bin/env python3
"""Compiler-free adversarial tests for the matrix's fail-closed ratchet."""
import contextlib
import copy
import io
import unittest

import check_sse2_codegen_matrix as matrix


class MatrixContractTests(unittest.TestCase):
    def setUp(self):
        self.row = matrix.ROWS[0]
        self.cfg = "O2"
        self.base = {
            "verdicts": {self.row.mnemonic: {self.cfg: "loss"}},
            "debt": {self.row.mnemonic: {self.cfg: {"lccc": 12, "best_oracle": 10}}},
            "mnemonics": {},
        }
        self.result = {"rows": {self.row.mnemonic: {"compilers": {
            "lccc": {self.cfg: {"status": "ok", "instructions": 12}},
            "gcc": {self.cfg: {"status": "ok", "instructions": 10, "hash_match": True}},
            "clang": {self.cfg: {"status": "ok", "instructions": 10, "hash_match": True}},
        }}}}

    def check(self, result=None, base=None):
        with contextlib.redirect_stdout(io.StringIO()):
            return matrix.check_baseline(
                self.result if result is None else result, [self.row], [self.cfg],
                ["lccc", "gcc", "clang"], base=self.base if base is None else base,
            )

    def test_unchanged_measurement_passes(self):
        self.assertEqual(self.check(), 0)

    def test_two_disagreeing_oracles_are_not_skipped_as_no_oracle(self):
        for oracle in ("gcc", "clang"):
            self.result["rows"][self.row.mnemonic]["compilers"][oracle][self.cfg].update(
                status="hash-mismatch", hash_match=False)
        self.assertNotEqual(self.check(), 0)

    def test_missing_measurements_fail_closed(self):
        for compiler in ("lccc", "gcc", "clang"):
            for status in ("compile-error", "no-body", "missing-hash", "run-error"):
                with self.subTest(compiler=compiler, status=status):
                    result = copy.deepcopy(self.result)
                    result["rows"][self.row.mnemonic]["compilers"][compiler][self.cfg]["status"] = status
                    self.assertNotEqual(self.check(result), 0)

    def test_a_missing_compiler_entry_is_not_a_success(self):
        del self.result["rows"][self.row.mnemonic]["compilers"]["lccc"]
        self.assertNotEqual(self.check(), 0)

    def test_winning_and_tied_rows_keep_their_own_count_ratchet(self):
        for verdict in ("win", "tie"):
            with self.subTest(verdict=verdict):
                base = copy.deepcopy(self.base)
                base["debt"] = {}
                base["verdicts"][self.row.mnemonic][self.cfg] = verdict
                base["counts"] = {self.row.mnemonic: {self.cfg: 11}}
                self.assertNotEqual(self.check(base=base), 0)


    def _with_mnemonics(self, recorded, current, instructions):
        base = copy.deepcopy(self.base)
        base["mnemonics"] = {self.row.mnemonic: {self.cfg: recorded}}
        result = copy.deepcopy(self.result)
        entry = result["rows"][self.row.mnemonic]["compilers"]["lccc"][self.cfg]
        entry["mnemonics"] = {m: 1 for m in current}
        entry["instructions"] = instructions
        return result, base

    def test_expected_mnemonic_loss_fails_even_with_a_shrink(self):
        # The shrink exemption covers incidental spill/repack spellings,
        # never the canonical instruction the row pins.
        self.assertTrue(self.row.expect, "probe row must name an expected mnemonic")
        canonical = self.row.expect[0]
        result, base = self._with_mnemonics([canonical, "movaps"], ["movaps"], 11)
        self.assertNotEqual(self.check(result, base), 0)

    def test_incidental_mnemonic_loss_with_a_shrink_passes(self):
        canonical = self.row.expect[0]
        result, base = self._with_mnemonics(
            [canonical, "movaps"], [canonical], 11)
        self.assertEqual(self.check(result, base), 0)

    def test_incidental_mnemonic_loss_without_a_shrink_fails(self):
        canonical = self.row.expect[0]
        result, base = self._with_mnemonics(
            [canonical, "movaps"], [canonical], 12)
        self.assertNotEqual(self.check(result, base), 0)


if __name__ == "__main__":
    unittest.main()
