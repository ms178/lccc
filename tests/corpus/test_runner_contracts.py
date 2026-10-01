#!/usr/bin/env python3
"""Behavioral contract tests for tests/corpus/run_clang_c_corpus.py.

Compiler-free: every subprocess is a mock.  These pin the verdict contracts
the PR #721 review showed were broken (crash-passing, spawn-before-skip,
dead flags, oracle misuse) plus the diagnostic-oracle semantics (Clang
`{{regex}}` template splicing, counts, active prefixes).

Run:  python3 -m unittest tests/corpus/test_runner_contracts.py
  or: python3 tests/corpus/test_runner_contracts.py
"""
from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "run_clang_c_corpus", HERE / "run_clang_c_corpus.py")
rcmod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rcmod)


class SpawnMock:
    """Deterministic fake compiler: scripted (rc, stderr) per call."""

    def __init__(self, script, timeout_script=()):
        self.script = list(script)
        self.timeout_script = set(timeout_script)
        self.calls = []

    def __call__(self, cmd, timeout):
        idx = len(self.calls)
        self.calls.append(list(cmd))
        if idx in self.timeout_script:
            raise rcmod.subprocess.TimeoutExpired(cmd, timeout)
        rc, stderr = self.script[min(idx, len(self.script) - 1)]
        return rc, b"", stderr.encode() if isinstance(stderr, str) else stderr


def rec(**kw):
    base = {
        "origin": "tests/tests/imported/clang/c/X.sft.c",
        "file": "clang-c/X.c", "category": "Sema", "language": "c",
        "languages_per_set": ["c"], "lccc_expect": "accept",
        "option_sets": [""], "gcc_flags_per_set": [[]],
        "options_all": [], "arch_tags": [], "expected": [],
        "phase": "compile", "untranslated_edg_flags": [],
        "verify_prefixes_per_set": [["expected"]],
    }
    base.update(kw)
    return base


class OutcomeClassificationTest(unittest.TestCase):
    def test_signal_negative_is_crash_not_reject(self):
        # PR#721 review P1-1: rc=-11 (SIGSEGV) counted as PASS for reject.
        self.assertEqual(rcmod.classify(-11, ""), rcmod.Outcome.CRASH)

    def test_shell_signal_rc_139_is_crash(self):
        self.assertEqual(rcmod.classify(139, ""), rcmod.Outcome.CRASH)

    def test_ice_text_is_crash_even_with_rc_1(self):
        self.assertEqual(rcmod.classify(1, "internal compiler error: boom"),
                         rcmod.Outcome.CRASH)

    def test_plain_error_rc_is_reject_observation(self):
        self.assertEqual(rcmod.classify(1, "x.c:1:1: error: nope"),
                         rcmod.Outcome.REJECT)

    def test_rc_zero_is_accept_observation(self):
        self.assertEqual(rcmod.classify(0, ""), rcmod.Outcome.ACCEPT)


class VerdictTest(unittest.TestCase):
    def _run(self, spawn, expect, **kw):
        return rcmod.run_one(rec(lccc_expect=expect, **kw), Path("."),
                             "cc", 5.0, True, True, "diagnostics", spawn=spawn)

    def test_crash_never_passes_reject(self):
        r = self._run(SpawnMock([(-11, "")]), "reject")
        self.assertEqual(r["outcome"], "CRASH")
        self.assertNotEqual(r["status"], "PASS")

    def test_crash_is_hard_fail_even_under_xfail(self):
        r = self._run(SpawnMock([(-11, "")]), "xfail")
        self.assertEqual(r["status"], "FAIL")
        self.assertIn("crash", r["detail"].lower())

    def test_clean_reject_with_no_messages_passes(self):
        r = self._run(SpawnMock([(1, "x.c:1:1: error: bad")]), "reject")
        self.assertEqual(r["status"], "PASS")

    def test_reject_expected_but_accepted_fails(self):
        r = self._run(SpawnMock([(0, "")]), "reject")
        self.assertEqual(r["status"], "FAIL")

    def test_timeout_is_hard_fail(self):
        r = self._run(SpawnMock([], timeout_script={0}), "reject")
        self.assertEqual(r["status"], "TIMEOUT")
        self.assertNotEqual(r["status"], "PASS")

    def test_xfail_semantic_mismatch_is_nonfatal(self):
        r = self._run(SpawnMock([(0, "")]), "xfail")
        self.assertEqual(r["status"], "XFAIL")

    def test_no_reject_skips_before_spawning(self):
        spawn = SpawnMock([(1, "error")])
        r = rcmod.run_one(rec(lccc_expect="reject"), Path("."), "cc",
                          5.0, True, False, "diagnostics", spawn=spawn)
        self.assertEqual(spawn.calls, [])
        self.assertEqual(r["status"], "SKIP")

    def test_xfail_as_pass_collapses_summary(self):
        spawn = SpawnMock([(0, "")])
        r = rcmod.run_one(rec(lccc_expect="xfail"), Path("."), "cc",
                          5.0, True, True, "diagnostics",
                          xfail_as_pass=True, spawn=spawn)
        self.assertEqual(r["status"], "PASS")

    def test_cpp_invocation_skipped_unsupported_but_c_set_runs(self):
        spawn = SpawnMock([(0, ""), (0, "")])
        r = rcmod.run_one(rec(lccc_expect="accept",
                              option_sets=["", "--c++"],
                              languages_per_set=["c", "c++"],
                              gcc_flags_per_set=[[], ["-std=c++11"]],
                              verify_prefixes_per_set=[["expected"],
                                                       ["expected"]]),
                          Path("."), "cc", 5.0, True, True,
                          "diagnostics", spawn=spawn)
        self.assertEqual(r["status"], "PASS")
        self.assertEqual(len(spawn.calls), 1)  # only the C set

    def test_options_all_applied_to_every_invocation(self):
        spawn = SpawnMock([(0, "")])
        rcmod.run_one(rec(options_all=["-DALL=1"], option_sets=["", ""],
                          gcc_flags_per_set=[[], []]), Path("."), "cc",
                      5.0, True, True, "diagnostics", spawn=spawn)
        self.assertEqual(len(spawn.calls), 2)
        for call in spawn.calls:
            self.assertIn("-DALL=1", call)

    def test_undecodable_stderr_does_not_abort(self):
        class Raw:
            def __call__(self, cmd, timeout):
                return 1, b"", b"\xff\xfe error"
        r = rcmod.run_one(rec(lccc_expect="reject"), Path("."), "cc",
                          5.0, True, True, "diagnostics", spawn=Raw())
        self.assertIn(r["status"], ("PASS", "FAIL"))  # structured, not an abort

    def test_untranslated_flags_reported(self):
        r = self._run(SpawnMock([(0, "")]), "accept",
                      untranslated_edg_flags=["--microsoft"])
        self.assertEqual(r["untranslated_edg_flags"], ["--microsoft"])


class DiagnosticOracleTest(unittest.TestCase):
    def test_template_regex_splicing(self):
        # PR#721 review P1-3: literal chunks escaped, {{...}} spliced raw.
        rx = rcmod.template_to_regex("'(unnamed struct at {{.*}})' cannot be")
        import re
        pat = re.compile(rx)
        self.assertTrue(pat.search(
            "error: '(unnamed struct at /x/n2350.c:31:10)' cannot be"))
        # parens in literal chunks must NOT become groups: a non-matching
        # text with a different literal must fail
        self.assertFalse(pat.search("error: 'other thing' cannot be"))

    def test_counts_enforced(self):
        exp = [{"kind": "warning", "prefix": "expected", "regex_form": False,
                "loc": "", "count": "2", "count_min": 2, "count_max": 2,
                "msg": "unused", "msg_regex": "unused", "line": 1}]
        ok, _ = rcmod.check_diagnostics(exp, ["expected"],
                                        "warning: unused\nwarning: unused\n")
        self.assertTrue(ok)
        ok, detail = rcmod.check_diagnostics(exp, ["expected"],
                                             "warning: unused\n")
        self.assertFalse(ok)

    def test_count_plus_form(self):
        exp = [{"kind": "warning", "prefix": "expected", "regex_form": True,
                "loc": "", "count": "1+", "count_min": 1,
                "count_max": float("inf"), "msg": "{{x}}",
                "msg_regex": "x", "line": 1}]
        ok, _ = rcmod.check_diagnostics(exp, ["expected"], "x x x")
        self.assertTrue(ok)
        ok, _ = rcmod.check_diagnostics(exp, ["expected"], "nothing")
        self.assertFalse(ok)

    def test_active_prefix_selection(self):
        exp = [{"kind": "warning", "prefix": "c23", "regex_form": False,
                "loc": "", "count": None, "count_min": 1, "count_max": 1,
                "msg": "std", "msg_regex": "std", "line": 1}]
        ok, _ = rcmod.check_diagnostics(exp, ["expected"], "nothing here")
        self.assertTrue(ok)          # inactive prefix: not enforced
        ok, _ = rcmod.check_diagnostics(exp, ["expected", "c23"],
                                        "nothing here")
        self.assertFalse(ok)         # active: must match

    def test_no_diagnostics_marker(self):
        exp = [{"kind": "no-diagnostics", "prefix": "expected",
                "regex_form": False, "loc": "", "count": None,
                "count_min": 0, "count_max": 0, "msg": "",
                "msg_regex": "", "line": 1}]
        ok, _ = rcmod.check_diagnostics(exp, ["expected"], "")
        self.assertTrue(ok)
        ok, _ = rcmod.check_diagnostics(exp, ["expected"], "error: x")
        self.assertFalse(ok)


class DifferentialOracleTest(unittest.TestCase):
    def test_differential_compares_outcomes_not_verdicts(self):
        # Candidate rejects-but-crashes must NOT be silently dropped while
        # the reference cleanly accepts: divergence = outcome inequality.
        c = rcmod.run_one(rec(lccc_expect="reject"), Path("."), "cc",
                          5.0, True, True, "diagnostics",
                          spawn=SpawnMock([(-11, "")]))
        r = rcmod.run_one(rec(lccc_expect="reject"), Path("."), "gcc",
                          5.0, True, True, "diagnostics",
                          spawn=SpawnMock([(0, "")]))
        self.assertNotEqual(c["outcome"], r["outcome"])
        self.assertTrue(rcmod.is_divergence(c, r))


class CliContractTest(unittest.TestCase):
    def test_invalid_category_is_usage_error(self):
        rc = rcmod.main(["--index", str(HERE / "clang-c" / "corpus-index.json"),
                         "--category", "NoSuchCategory", "--cc", "true"])
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main(verbosity=1)
