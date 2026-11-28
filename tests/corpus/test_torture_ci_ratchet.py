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
        # The release `runs-on` asks for is the workflow's choice, not the
        # provider's: another one is another experiment.
        changed = copy.deepcopy(self.data)
        changed["host_image"]["os"] = "ubuntu26"
        self.assertEqual(ratchet.compare(changed, self.baseline), ["identity mismatch: host_image.os"])
        # Every key of the identity is either one that refuses or one of the
        # two that say where it ran: a key added to the provenance refuses
        # until somebody decides otherwise here.
        for key in self.baseline["comparison"]:
            with self.subTest(key=key):
                changed = copy.deepcopy(self.baseline)
                changed["comparison"][key] = "another"
                if key in ratchet.HOST_KEYS:
                    continue
                self.assertEqual(ratchet.compare(self.data, changed), [f"identity mismatch: {key}"])
        extended = copy.deepcopy(self.baseline)
        extended["comparison"]["new_knob"] = 1
        self.assertEqual(ratchet.compare(self.data, extended), ["identity mismatch: new_knob"])

    # The two fleets one `ubuntu-24.04` label was served from in November
    # 2026, as the hosted legs reported them.
    AWS = ("Linux-7.0.0-1014-aws-x86_64-with-glibc2.39", "20261116.133.1")
    AZURE = ("Linux-7.0.0-1017-azure-x86_64-with-glibc2.39", "20261116.376.1")

    def on(self, fleet, data=None):
        moved = copy.deepcopy(self.data if data is None else data)
        moved["host"], moved["host_image"]["version"] = fleet
        return moved

    def captured_on(self, fleet):
        data = self.on(fleet)
        return {"schema": 1, "source_run": 123, "source_head": "b" * 40,
                "comparison": provenance.provenance(data)["comparison"],
                "counts": data["counts"].copy(), "outcomes": provenance.outcome_evidence(data)}

    def test_where_it_ran_is_reported_and_the_rows_are_compared_all_the_same(self):
        # The red check of main, 2026-11-19 to 2026-11-25: the i686 baseline
        # was captured on one fleet, the x86_64 one on the other, and a leg
        # that drew the other fleet failed with nothing changed.
        for captured, ran in ((self.AWS, self.AZURE), (self.AZURE, self.AWS)):
            with self.subTest(captured=captured, ran=ran):
                baseline = self.captured_on(captured)
                self.assertEqual(ratchet.compare(self.on(captured), baseline), [])
                self.assertEqual(ratchet.host_drift(self.on(captured), baseline), [])
                self.assertEqual(ratchet.compare(self.on(ran), baseline), [])
                self.assertEqual(ratchet.host_drift(self.on(ran), baseline),
                                 [f"host: {captured[0]} -> {ran[0]}",
                                  f"host_image.version: {captured[1]} -> {ran[1]}"])
        # A new build of the image on the same kernel is one note.
        baseline = self.captured_on(self.AZURE)
        newer = self.on((self.AZURE[0], "20261123.380.1"))
        self.assertEqual(ratchet.compare(newer, baseline), [])
        self.assertEqual(ratchet.host_drift(newer, baseline),
                         ["host_image.version: 20261116.376.1 -> 20261123.380.1"])

    def test_a_changed_row_on_another_host_is_named_and_not_hidden_behind_the_host(self):
        # Runs 45174788820 to 45388535959: three cases had gone from
        # compile-fail to pass and the ratchet said "identity mismatch:
        # host_image", which is all it said.
        baseline = self.captured_on(self.AWS)
        fixed = self.on(self.AZURE)
        fixed["results"][1]["status"] = "pass"
        problems = ratchet.compare(fixed, baseline)
        self.assertEqual(problems, ['["fail.c", "-O2"]: compile-fail -> pass'])
        self.assertTrue(ratchet.host_drift(fixed, baseline))
        broken = self.on(self.AZURE)
        broken["results"][0]["status"] = "compile-fail"
        self.assertEqual(ratchet.compare(broken, baseline), ['["pass.c", "-O2"]: pass -> compile-fail'])
        changed = self.on(self.AZURE)
        changed["results"][2]["detail"] = "another header on this image"
        self.assertIn("diagnostic changed", ratchet.compare(changed, baseline)[0])

    def test_a_baseline_says_where_it_was_captured(self):
        for mutation in (lambda c: c.pop("host"),
                         lambda c: c.update(host=""),
                         lambda c: c.update(host=7),
                         lambda c: c.pop("host_image"),
                         lambda c: c.update(host_image="ubuntu24"),
                         lambda c: c["host_image"].pop("version"),
                         lambda c: c["host_image"].update(version=""),
                         lambda c: c["host_image"].update(os=None),
                         lambda c: c["host_image"].update(kernel="7.0")):
            bad = copy.deepcopy(self.baseline)
            mutation(bad["comparison"])
            for use in (ratchet.compare, ratchet.host_drift):
                with self.assertRaises(ValueError):
                    use(self.data, bad)

    def test_the_checked_in_baselines_are_well_formed_and_of_one_experiment(self):
        directory = ROOT / "engineering/evidence/gcc17-torture-baseline"
        strict = {}
        for arch in ("i686", "x86_64"):
            for mode in ("compile", "execute"):
                baseline = json.loads((directory / f"{arch}-{mode}.json").read_text())
                comparison = ratchet.baseline_identity(baseline)
                self.assertEqual((comparison["arch"], comparison["mode"]), (arch, mode))
                strict[arch, mode] = ratchet.what_ran(comparison)
                self.assertNotIn("host", strict[arch, mode])
                self.assertNotIn("host_image", strict[arch, mode])
        # What does not depend on the target or the mode is the same in all
        # four: one corpus, one runner, one oracle environment, one release.
        for key in ("corpus", "corpus_stamp_sha256", "runner_sha256", "harness_dependency_sha256",
                    "oracle_environment_sha256", "host_image.os", "flags", "jobs", "link_mode"):
            self.assertEqual(len({json.dumps(value[key], sort_keys=True) for value in strict.values()}), 1, key)

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

    def test_error_annotation_bounds_and_escapes_untrusted_names(self):
        payload = ratchet.error_annotation(["case%\n::error::" + "x" * 100] * 100)
        self.assertLessEqual(len(payload.encode()), 3800)
        self.assertNotIn("\n", payload)
        self.assertIn("%25", payload)
        decoded = json.loads(payload.replace("%25", "%"))
        self.assertGreater(decoded["omitted"], 0)
        self.assertEqual(len(decoded["problems"]) + decoded["omitted"], 100)

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

    def test_cli_on_another_host_passes_with_a_notice_and_fails_on_a_changed_row(self):
        with tempfile.TemporaryDirectory() as temp:
            report, baseline = Path(temp) / "report.json", Path(temp) / "baseline.json"
            baseline.write_text(json.dumps(self.captured_on(self.AWS)))
            def check(data, hosted=True):
                report.write_text(json.dumps(data))
                env = {"PATH": "/usr/bin:/bin", **({"GITHUB_ACTIONS": "true"} if hosted else {})}
                return subprocess.run([sys.executable, str(ROOT / "scripts/torture_ci_ratchet.py"),
                                       str(report), str(baseline)], capture_output=True, text=True,
                                      timeout=5, env=env)
            same = check(self.on(self.AWS))
            self.assertEqual(same.returncode, 0, same.stderr)
            self.assertNotIn("::notice", same.stdout)
            self.assertNotIn("another host", same.stdout)
            moved = check(self.on(self.AZURE))
            self.assertEqual(moved.returncode, 0, moved.stderr)
            self.assertNotIn("::error", moved.stdout)
            notices = [line for line in moved.stdout.splitlines() if line.startswith("::notice ")]
            self.assertEqual(len(notices), 1, moved.stdout)
            title, _, payload = notices[0].partition("::notice title=")[2].partition("::")
            self.assertEqual(title, "GCC torture ratchet host")
            self.assertEqual(json.loads(payload),
                             {"arch": "x86_64", "mode": "compile",
                              "host_drift": [f"host: {self.AWS[0]} -> {self.AZURE[0]}",
                                             f"host_image.version: {self.AWS[1]} -> {self.AZURE[1]}"]})
            self.assertIn("matches reviewed per-case baseline", moved.stdout)
            local = check(self.on(self.AZURE), hosted=False)
            self.assertEqual(local.returncode, 0, local.stderr)
            self.assertNotIn("::notice", local.stdout)
            self.assertIn('  "host_image.version: 20261116.133.1 -> 20261116.376.1"\n', local.stdout)
            fixed = self.on(self.AZURE)
            fixed["results"][1]["status"] = "pass"
            fixed["counts"] = dict(Counter(r["status"] for r in fixed["results"]))
            failed = check(fixed)
            self.assertEqual(failed.returncode, 1)
            self.assertIn("::notice title=GCC torture ratchet host::", failed.stdout)
            errors = [line for line in failed.stdout.splitlines() if line.startswith("::error ")]
            self.assertEqual(len(errors), 1, failed.stdout)
            self.assertEqual(json.loads(errors[0].partition("::error title=GCC torture ratchet::")[2]),
                             {"problems": ['["fail.c", "-O2"]: compile-fail -> pass'], "omitted": 0})
            # What the provider reports goes into a workflow command: a line
            # break in it must not start a second one.
            hostile = self.on(("Linux\n::error::owned%0A", "1\r2"))
            escaped = check(hostile)
            self.assertEqual(escaped.returncode, 0, escaped.stderr)
            commands = [line for line in escaped.stdout.splitlines() if "::" in line]
            self.assertEqual(len(commands), 2, escaped.stdout)
            self.assertTrue(commands[0].startswith("::notice title=GCC torture ratchet host::"))
            self.assertNotIn("%0A", commands[0])
            self.assertIn("%250A", commands[0])
            self.assertTrue(commands[1].startswith('  "host: '), commands[1])
            self.assertEqual(len(escaped.stdout.splitlines()), 5, escaped.stdout)


if __name__ == "__main__":
    unittest.main()
