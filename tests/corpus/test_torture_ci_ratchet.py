"""Failure injection for the hosted per-case torture ratchet."""
from collections import Counter
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT))
import torture_ci_provenance as provenance  # noqa: E402
import torture_ci_ratchet as ratchet  # noqa: E402
from tools.corpus.evidence import case_set_digest  # noqa: E402


class Ratchet(unittest.TestCase):
    def setUp(self):
        rows = [{"test": "pass.c", "flags": "-O2", "status": "pass"},
                {"test": "fail.c", "flags": "-O2", "status": "compile-fail"},
                {"test": "skip.c", "flags": "-O2", "status": "reference-compile-skip"},
                {"test": "unsupported.c", "flags": "-O2", "status": "unsupported"}]
        for row in rows:
            row.update(phase="compile", returncode=1, detail="test diagnostic")
        self.data = {"schema": 2, "complete": True, "fatal": None, "mode": "compile",
                     "arch": "x86_64", "gcc_checkout_head": "a" * 40,
                     "expected_cases": 4, "completed_cases": 4, "flags": ["-O2"],
                     "results": rows, "counts": dict(Counter(r["status"] for r in rows)),
                     "case_set_sha256": case_set_digest(f"{r['test']}[{r['flags']}]" for r in rows),
                     "selection": {"tests": [], "filter": "", "from_list": None, "sources": 4,
                                   "selected_sources_sha256": "a" * 64, "dependency_files": 0,
                                   "dependency_digest_truncated": False},
                     "dependency_digest": "b" * 64, "corpus_stamp_sha256": "c" * 64,
                     "runner_sha256": "d" * 64, "harness_dependency_sha256": "2" * 64,
                     "oracle_environment_sha256": "e" * 64,
                     "host_image": {"os": "ubuntu24", "version": "20261101.1"}, "host": "Linux",
                     "tools": {"gcc": {"sha256": "f" * 64, "version_returncode": 0},
                               "lccc": {"sha256": "1" * 64}},
                     "link_mode": "lccc-ld", "append": "", "runner": "", "expensive": False,
                     "compile_timeout": 180, "run_timeout": 10, "jobs": 2, "environment": {}}
        self.baseline = {"schema": 1, "source_run": 123, "source_head": "b" * 40,
                         "comparison": provenance.provenance(self.data)["comparison"],
                         "counts": self.data["counts"].copy(),
                         "outcomes": provenance.outcome_evidence(self.data)}

    def test_exact_match_and_candidate_binary_changes(self):
        self.assertEqual(ratchet.compare(self.data, self.baseline), [])
        self.data["tools"]["lccc"]["sha256"] = "2" * 64
        self.assertEqual(ratchet.compare(self.data, self.baseline), [])

    def test_new_failure_skip_changes_reference_fail_and_fixed_case_are_detected(self):
        for index, status in ((0, "compile-fail"), (0, "reference-compile-skip"),
                              (1, "pass"), (1, "reference-compile-skip"),
                              (2, "pass"), (2, "reference-fail"), (3, "pass")):
            with self.subTest(index=index, status=status):
                changed = copy.deepcopy(self.data)
                changed["results"][index]["status"] = status
                self.assertTrue(ratchet.compare(changed, self.baseline))

    def test_same_status_with_changed_cause_exit_or_phase_requires_review(self):
        for field, value in (("detail", "internal compiler error"), ("returncode", -11),
                             ("phase", "link")):
            for index in (1, 2, 3):
                changed = copy.deepcopy(self.data)
                changed["results"][index][field] = value
                problems = ratchet.compare(changed, self.baseline)
                self.assertTrue(problems)
                self.assertIn("diagnostic changed", problems[0])

    def test_identity_drift_is_refused_not_advisory(self):
        for field in ("runner_sha256", "harness_dependency_sha256", "dependency_digest",
                      "corpus_stamp_sha256", "oracle_environment_sha256"):
            with self.subTest(field=field):
                changed = copy.deepcopy(self.data)
                changed[field] = "0" * 64
                self.assertIn("identity mismatch", ratchet.compare(changed, self.baseline)[0])
        changed = copy.deepcopy(self.data)
        changed["tools"]["gcc"]["sha256"] = "0" * 64
        self.assertTrue(ratchet.compare(changed, self.baseline))
        changed = copy.deepcopy(self.data)
        changed["selection"]["filter"] = "pass"
        with self.assertRaises(ValueError):
            ratchet.compare(changed, self.baseline)

    def test_bad_baseline_counts_duplicates_and_reference_fail_refused(self):
        for mutation in (lambda b: b["counts"].update(pass_=9),
                         lambda b: b["outcomes"].append(b["outcomes"][0]),
                         lambda b: b["outcomes"][0].__setitem__(2, "reference-fail"),
                         lambda b: b["outcomes"].pop(),
                         lambda b: b.update(source_head="invalid"),
                         lambda b: b.update(schema=True)):
            bad = copy.deepcopy(self.baseline)
            mutation(bad)
            with self.assertRaises(ValueError):
                ratchet.compare(self.data, bad)

    def test_cli_rejects_incomplete_duplicate_and_new_failure(self):
        with tempfile.TemporaryDirectory() as temp:
            report, baseline = Path(temp) / "report.json", Path(temp) / "baseline.json"
            baseline.write_text(json.dumps(self.baseline))
            def check(data):
                report.write_text(json.dumps(data))
                return subprocess.run([sys.executable, str(ROOT / "scripts/torture_ci_ratchet.py"),
                                       str(report), str(baseline)], capture_output=True, text=True, timeout=5)
            self.assertEqual(check(self.data).returncode, 0)
            changed = copy.deepcopy(self.data)
            changed["results"][0]["status"] = "compile-fail"
            changed["counts"] = dict(Counter(r["status"] for r in changed["results"]))
            self.assertEqual(check(changed).returncode, 1)
            changed["complete"] = False
            self.assertEqual(check(changed).returncode, 2)
            changed = copy.deepcopy(self.data)
            changed["results"].append(changed["results"][0])
            self.assertEqual(check(changed).returncode, 2)


if __name__ == "__main__":
    unittest.main()
