#!/usr/bin/env python3
"""Mutation tests for scripts/c_reduce.py and scripts/c_reduce_predicate.py (T4).

Run: python3 -m unittest discover -s tests/fuzz -p 'test_c_reduce.py'

Each test is a mutation check: it fails if the reducer or predicate is broken
in the way it is meant to catch.  The end-to-end cases use a fake compiler
with a known miscompile (tests/fuzz/fake_miscompiler.py), so the expected
witness is known in advance.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import c_reduce  # noqa: E402

FAKE = HERE / "fake_miscompiler.py"
PRED = ROOT / "scripts" / "c_reduce_predicate.py"
CLI = ROOT / "scripts" / "c_reduce.py"

PROGRAM = """\
#include <stdio.h>
static int helper_a(int v) { return v + 11; }
static int helper_b(int v) { return v ^ 0x55; }
static int unused_table[8] = {1, 2, 3, 4, 5, 6, 7, 8};
int main(void) {
    int acc = 0;
    for (int i = 0; i < 4; i++) acc += helper_a(i);
    acc = helper_b(acc);
    int x = 7;
    printf("noise %d\\n", acc + unused_table[2]);
    printf("value %d\\n", x * 3);
    return 0;
}
"""


def have_gcc() -> bool:
    return shutil.which(os.environ.get("REF_CC", "gcc")) is not None


class DdminUnit(unittest.TestCase):
    def test_result_is_one_minimal_for_two_needed_lines(self):
        """ddmin keeps exactly the two lines the predicate needs, no more."""
        lines = [f"line {i}\n" for i in range(40)]
        lines[7] = "NEEDED-A\n"
        lines[31] = "NEEDED-B\n"
        with tempfile.TemporaryDirectory() as tmp:
            def check(path: Path) -> bool:
                text = path.read_text()
                return "NEEDED-A" in text and "NEEDED-B" in text
            red = c_reduce.Reducer(check, Path(tmp), 10_000, lines)
            out = red.one_minimal(red.ddmin(lines))
        self.assertEqual(sorted(out), ["NEEDED-A\n", "NEEDED-B\n"])

    def test_budget_returns_accepted_progress_not_input(self):
        """A budget stop must keep reductions already accepted (mutation: returning the input)."""
        lines = [f"x{i}\n" for i in range(64)]
        lines[0] = "KEEP\n"
        with tempfile.TemporaryDirectory() as tmp:
            def check(path: Path) -> bool:
                return "KEEP" in path.read_text()
            out, calls, exhausted = c_reduce.reduce_text(
                "".join(lines), check, max_tests=3, scratch=Path(tmp))
        self.assertTrue(exhausted)
        self.assertLessEqual(calls, 3)
        self.assertIn("KEEP", out)
        self.assertLess(len(out.splitlines()), len(lines))

    def test_memoisation_skips_repeat_candidates(self):
        """Identical candidates are answered from the cache, not re-run."""
        lines = ["a\n", "b\n", "c\n", "d\n"]
        with tempfile.TemporaryDirectory() as tmp:
            red = c_reduce.Reducer(lambda p: "a" in p.read_text(), Path(tmp), 100, lines)
            red.interesting(["a\n"])
            red.interesting(["a\n"])
            self.assertEqual(red.tests, 1)


@unittest.skipUnless(have_gcc(), "needs a reference C compiler")
class EndToEnd(unittest.TestCase):
    def _run(self, env_extra, body=PROGRAM):
        tmp = tempfile.mkdtemp(prefix="c-reduce-e2e-")
        self.addCleanup(shutil.rmtree, tmp, True)
        src = Path(tmp) / "case.c"
        src.write_text(body)
        out = Path(tmp) / "min.c"
        env = dict(os.environ, **env_extra)
        proc = subprocess.run(
            [sys.executable, str(CLI), str(src), "--out", str(out), "--",
             sys.executable, str(PRED), "--ccc", str(FAKE), "--level=-O2"],
            env=env, capture_output=True, text=True, timeout=600, check=False)
        return proc, out

    def test_reduces_known_miscompile_to_a_small_witness(self):
        proc, out = self._run({"FAKE_MISCOMPILE": "1"})
        self.assertEqual(proc.returncode, 0, proc.stderr)
        reduced = out.read_text()
        self.assertIn("* 3", reduced)
        self.assertLessEqual(len(reduced.splitlines()), 8, reduced)
        self.assertLess(len(reduced), len(PROGRAM) // 2)

    def test_correct_compiler_is_not_interesting(self):
        """Negative control: a correct compiler gives exit 2, no reduction."""
        proc, out = self._run({"FAKE_MISCOMPILE": "0"})
        self.assertEqual(proc.returncode, 2, proc.stderr)
        self.assertFalse(out.exists())

    def test_ub_candidate_is_rejected_by_reference_oracle(self):
        """A program with out-of-bounds access must not be blamed on the compiler.

        The fake miscompiles, but the reference (ASan) rejects the input first.
        """
        ub = PROGRAM.replace("unused_table[2]", "unused_table[99]")
        proc, _ = self._run({"FAKE_MISCOMPILE": "1"}, body=ub)
        self.assertEqual(proc.returncode, 2, proc.stderr)


class PredicateContract(unittest.TestCase):
    def test_predicate_usage_error_aborts_instead_of_meaning_not_interesting(self):
        """Mutation: a predicate exiting 2 must abort the reduction (exit 1), not be read as a verdict."""
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "c.c"
            src.write_text("int main(void){return 0;}\n")
            out = Path(tmp) / "o.c"
            proc = subprocess.run(
                [sys.executable, str(CLI), str(src), "--out", str(out), "--",
                 sys.executable, "-c", "import sys; sys.exit(2)"],
                capture_output=True, text=True, timeout=120, check=False)
            self.assertEqual(proc.returncode, 1, proc.stderr)
            self.assertIn("expected 0 or 1", proc.stderr)
            self.assertFalse(out.exists())


    def test_missing_compiler_under_test_is_an_error_not_a_crash(self):
        """Mutation: a missing `--ccc` must not count as a crash (that reduces to nothing)."""
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "c.c"
            src.write_text("int main(void){return 0;}\n")
            proc = subprocess.run(
                [sys.executable, str(PRED), "--ccc", str(Path(tmp) / "no-such-cc"),
                 "--kind", "crash", str(src)],
                capture_output=True, text=True, timeout=120, check=False)
            self.assertEqual(proc.returncode, 2, proc.stderr)
            self.assertIn("cannot execute", proc.stderr)


if __name__ == "__main__":
    unittest.main()
