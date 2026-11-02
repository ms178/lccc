#!/usr/bin/env python3
"""Red-team the phi fuzzer's verdicts without invoking either compiler."""
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import phi_cfg_fuzz


class ExecutionVerdictTests(unittest.TestCase):
    def run_case(self, reference, candidate):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # one() compiles reference, candidate, then executes both.
            with patch.object(phi_cfg_fuzz, "call", side_effect=[
                (0, "", ""), (0, "", ""), reference, candidate
            ]) as runner:
                result = phi_cfg_fuzz.one((0, "O2", "lccc", "gcc", root))
                self.assertEqual(runner.call_count, 4)
                commands = [c.args[0] for c in runner.call_args_list]
                self.assertEqual([c[0] for c in commands[:2]], ["gcc", "lccc"])
                self.assertEqual([Path(c[0]).name for c in commands[2:]], ["g", "c"])
            retained = (root / "00000-O2" / "x.c").exists()
            return result, retained

    def test_success_requires_zero_exit_and_equal_stdout(self):
        result, retained = self.run_case((0, "ok\n", ""), (0, "ok\n", ""))
        self.assertEqual(result["status"], "pass")
        self.assertFalse(retained)

    def test_equal_failures_never_pass_and_keep_reproducer(self):
        for code in (1, 139, -11, "TIMEOUT"):
            with self.subTest(code=code):
                result, retained = self.run_case((code, "", ""), (code, "", ""))
                self.assertEqual(result["status"], "both-fail")
                self.assertTrue(retained)

    def test_one_sided_failure_is_runtime_failure(self):
        for a, b in ((0, -11), (-11, 0), (0, "TIMEOUT")):
            with self.subTest(a=a, b=b):
                result, retained = self.run_case((a, "ok", ""), (b, "ok", ""))
                self.assertEqual(result["status"], "reference-fail" if a else "candidate-fail")
                self.assertTrue(retained)

    def test_output_mismatch_keeps_reproducer(self):
        result, retained = self.run_case((0, "a", ""), (0, "b", ""))
        self.assertEqual(result["status"], "mismatch")
        self.assertTrue(retained)


if __name__ == "__main__":
    unittest.main()
