"""Offline tests for deriving reviewed baselines from GitHub annotations."""
import copy
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT))
import torture_ci_capture as capture  # noqa: E402


class Capture(unittest.TestCase):
    def setUp(self):
        self.run = {"id": 123, "head_sha": "a" * 40, "status": "completed", "conclusion": "success",
                    "path": ".github/workflows/gcc17-torture.yml", "repository": {"full_name": "ms178/lccc"}}
        self.jobs = {"jobs": []}
        self.annotations = {}
        for number, arch in enumerate(capture.ARCHES, 1):
            steps = [{"name": name, "conclusion": "success"} for name in (*capture.REQUIRED_STEPS, capture.RATCHET)]
            self.jobs["jobs"].append({"id": number, "run_id": 123, "head_sha": "a" * 40,
                                      "name": f"GCC 17 torture — {arch} / GCC 17 torture — {arch}",
                                      "status": "completed", "conclusion": "success", "steps": steps})
            annotations = []
            for mode in capture.MODES:
                identity = {"arch": arch, "mode": mode, "corpus": "b" * 40, "flags": ["-O2"],
                            "case_set_sha256": "c" * 64, "expected_cases": 2}
                inventory = {**identity, "counts": {"pass": 1, "compile-fail": 1},
                             "nonpass": [["fail.c", "-O2", "compile-fail"]], "nonpass_omitted": 0}
                evidence = {"arch": arch, "mode": mode, "part": 1, "parts": 1,
                            "rows": [["fail.c", "-O2", "compile-fail", "compile", 1, "d" * 64]]}
                for title, value in (("GCC torture inventory", inventory),
                                     ("GCC torture provenance", {"comparison": identity}),
                                     ("GCC torture outcome evidence", evidence)):
                    annotations.append({"title": title, "message": json.dumps(value)})
            self.annotations[number] = annotations

    def test_all_four_baselines_and_provenance_are_preserved(self):
        data = capture.capture(self.run, self.jobs, self.annotations)
        self.assertEqual(set(data), {f"{a}-{m}.json" for a in capture.ARCHES for m in capture.MODES})
        self.assertTrue(all(d["source_run"] == 123 and d["source_head"] == "a" * 40 for d in data.values()))

    def test_missing_or_corrupted_evidence_is_not_a_baseline(self):
        for mutation in (
            lambda a: a[1].pop(),
            lambda a: a[1].append(copy.deepcopy(a[1][0])),
            lambda a: a[1][0].update(message=None),
            lambda a: a[1][0].update(message=a[1][0]["message"].replace('"nonpass_omitted": 0', '"nonpass_omitted": 1')),
            lambda a: a[1][2].update(message=a[1][2]["message"].replace('"parts": 1', '"parts": 2')),
            lambda a: a[1][2].update(message=a[1][2]["message"].replace('compile-fail', 'reference-fail')),
        ):
            with self.subTest(mutation=mutation):
                annotations = copy.deepcopy(self.annotations)
                mutation(annotations)
                with self.assertRaises(ValueError):
                    capture.capture(self.run, self.jobs, annotations)

    def test_failure_requires_explicit_opt_in_and_complete_error_evidence(self):
        self.run["conclusion"] = "failure"
        job = self.jobs["jobs"][0]
        job["conclusion"] = "failure"
        job["steps"][-1]["conclusion"] = "failure"
        with self.assertRaises(ValueError):
            capture.capture(self.run, self.jobs, self.annotations)
        with self.assertRaises(ValueError):
            capture.capture(self.run, self.jobs, self.annotations, allow_ratchet_failure=True)
        self.annotations[1].append({"title": "GCC torture ratchet",
                                    "message": json.dumps({"problems": ["identity mismatch"], "omitted": 0})})
        self.assertEqual(len(capture.capture(self.run, self.jobs, self.annotations, allow_ratchet_failure=True)), 4)
        self.annotations[1][-1]["message"] = json.dumps({"problems": ["identity mismatch"], "omitted": 1})
        with self.assertRaises(ValueError):
            capture.capture(self.run, self.jobs, self.annotations, allow_ratchet_failure=True)

    def test_wrong_run_head_or_failed_campaign_is_rejected(self):
        for mutation in (lambda j: j["jobs"][0].update(head_sha="b" * 40),
                         lambda j: j["jobs"][0].update(run_id=999),
                         lambda j: j["jobs"][0]["steps"][0].update(conclusion="failure"),
                         lambda j: j["jobs"].pop()):
            jobs = copy.deepcopy(self.jobs)
            mutation(jobs)
            with self.assertRaises(ValueError):
                capture.capture(self.run, jobs, self.annotations)


if __name__ == "__main__":
    unittest.main()
