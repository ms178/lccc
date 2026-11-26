#!/usr/bin/env python3
"""Pure-logic tests for the SQLite speedtest1 result/identity gate."""
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import compare_clang
import paired_speedtest

module = Path(__file__).with_name("summarize.py")
spec = importlib.util.spec_from_file_location("sqlite_speedtest_summarize", module)
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)


def output(number=100, answer="a" * 48, test="case", duration="0.500"):
    return (f"  100 - {test}........................    {duration}s\n"
            f"       TOTAL............................    {duration}s\n"
            f"Verification Hash: {number} {answer}\n")


class SpeedtestSummary(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.artifacts = Path(self.temp.name)

    def put(self, compiler, text):
        path = self.artifacts / f"bench-main-1-{compiler}.txt"
        path.write_text(text)
        return path

    def test_equal_outputs_accept_timing_noise(self):
        self.put("gcc", output())
        self.put("lccc", output(duration="0.600"))
        result = summary.summarize(self.artifacts, ["main"], 1)["main"]
        self.assertEqual(result["test_count"], 1)
        self.assertEqual(result["verification_hash"], "a" * 48)
        self.assertAlmostEqual(result["lccc_over_gcc_median_ratio_vm_only"], 1.2)

    def test_different_hash_is_fatal_even_when_time_is_equal(self):
        self.put("gcc", output())
        self.put("lccc", output(answer="b" * 48))
        with self.assertRaisesRegex(ValueError, "result mismatch"):
            summary.summarize(self.artifacts, ["main"], 1)

    def test_different_test_identity_is_fatal_even_when_hash_is_equal(self):
        self.put("gcc", output())
        self.put("lccc", output(test="different"))
        with self.assertRaisesRegex(ValueError, "test identities"):
            summary.summarize(self.artifacts, ["main"], 1)

    def test_missing_total_or_verification_is_fatal(self):
        p = self.put("gcc", output().replace("TOTAL", "GONE"))
        with self.assertRaisesRegex(ValueError, "missing total"):
            summary.parse_run(p)
        p.write_text(output().replace("Verification Hash:", "Gone:"))
        with self.assertRaisesRegex(ValueError, "missing total"):
            summary.parse_run(p)

    def test_compiler_digest_streamed_from_real_bytes(self):
        path = self.artifacts / "compiler"
        path.write_bytes(bytes(range(256)) * 4096)
        self.assertEqual(summary.compiler_sha256(str(path)), hashlib.sha256(path.read_bytes()).hexdigest())

    def test_paired_run_normalizes_only_json_test_pairs(self):
        path = self.put("lccc", output())
        run = summary.parse_run(path)
        manifest = {"verification_hash": "a" * 48, "verified_bytes": 100,
                    "test_count": 1, "runs": {"gcc": [{"tests": [[100, "case"]]}]}}
        paired_speedtest.validate_run(run, manifest, path)
        manifest["runs"]["gcc"][0]["tests"][0][1] = "other"
        with self.assertRaisesRegex(ValueError, "identities differ"):
            paired_speedtest.validate_run(run, manifest, path)
        manifest["runs"]["gcc"][0]["tests"][0][1] = "case"
        manifest["verification_hash"] = "b" * 48
        with self.assertRaisesRegex(ValueError, "hash or test identities"):
            paired_speedtest.validate_run(run, manifest, path)

    def test_paired_run_rejects_runtime_stderr_even_with_matching_hash(self):
        log = self.artifacts / "speedtest.err"
        paired_speedtest.require_silent(b"", log)
        with self.assertRaisesRegex(ValueError, "unexpected runtime stderr"):
            paired_speedtest.require_silent(b"warning: disk issue\n", log)

    def test_clang_warning_scan_rejects_diagnostics(self):
        log = self.artifacts / "build-clang.log"
        log.write_text("compile passed\n")
        compare_clang.no_warnings([log])
        log.write_text("foo.c:12: WARNING: unexpected diagnostic\n")
        with self.assertRaisesRegex(ValueError, "WARNING"):
            compare_clang.no_warnings([log])

    def test_clang_runtime_stderr_and_nonzero_status_fail_even_with_valid_stdout(self):
        log = self.artifacts / "bench-main.txt"
        good = output()
        for stderr, code in [("", 0), ("warning: slow disk\n", 0), ("", 5)]:
            script = ("import sys; sys.stdout.write(" + repr(good) + "); "
                      "sys.stderr.write(" + repr(stderr) + "); sys.exit(" + str(code) + ")")
            command = [sys.executable, "-c", script]
            if stderr or code:
                with self.assertRaisesRegex(RuntimeError, "speedtest exited"):
                    compare_clang.run_speedtest(command, log, cwd=self.artifacts)
            else:
                compare_clang.run_speedtest(command, log, cwd=self.artifacts)
            self.assertEqual(log.read_text(), good)
            self.assertEqual(log.with_suffix(".err").read_text(), stderr)

    def test_integrity_gate_rejects_nonzero_exit_stderr_and_bad_stdout(self):
        helper = Path(__file__).with_name("integrity_gate.sh")
        cli = self.artifacts / "fake-sqlite3"
        cli.write_text("#!/bin/sh\nprintf '%s' \"$FAKE_STDOUT\"\n"
                       "printf '%s' \"$FAKE_STDERR\" >&2\nexit \"$FAKE_EXIT\"\n")
        cli.chmod(0o755)
        out = self.artifacts / "integrity.txt"
        err = self.artifacts / "integrity.err"
        command = ["bash", "-c", "set -euo pipefail; source \"$1\"; "
                   "sqlite_speedtest_integrity \"$2\" \"$3\" \"$4\" \"$5\"",
                   "bash", str(helper), str(cli), str(self.artifacts / "test.db"),
                   str(out), str(err)]
        for stdout, stderr, code, accepted in [
            ("ok\n", "", 0, True),
            ("ok\n", "", 7, False),
            ("ok\n", "warning: disk\n", 0, False),
            ("corrupt\n", "", 0, False),
        ]:
            env = dict(os.environ, FAKE_STDOUT=stdout, FAKE_STDERR=stderr,
                       FAKE_EXIT=str(code))
            proc = subprocess.run(command, env=env, capture_output=True, timeout=5)
            self.assertEqual(proc.returncode == 0, accepted,
                             f"stdout={stdout!r}, stderr={stderr!r}, exit={code}")
            self.assertEqual(out.read_text(), stdout)
            self.assertEqual(err.read_text(), stderr)

    def test_workload_early_failure_invalidates_previous_pass_manifest(self):
        output_dir = self.artifacts / "workload"
        output_dir.mkdir()
        report = output_dir / "manifest.json"
        report.write_text('{"status": "PASS"}')
        script = Path(__file__).with_name("run.sh")
        env = dict(os.environ, SQLITE_ARTIFACT_DIR=str(output_dir),
                   LCCC=str(self.artifacts / "missing-compiler"),
                   LCCC_LD=str(self.artifacts / "missing-linker"))
        result = subprocess.run(["bash", str(script)], env=env,
                                capture_output=True, timeout=20)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(report.exists(), "prerequisite failures cannot leave old PASS")

    def test_failed_clang_rerun_invalidates_old_pass_manifest(self):
        baseline = self.artifacts / "baseline"
        baseline.mkdir()
        (baseline / "manifest.json").write_text(json.dumps({
            "status": "PASS", "version": "3.54.1", "fossil_manifest_uuid": "pinned",
        }))
        work = self.artifacts / "work"
        (work / "sqlite-version-3.54.1").mkdir(parents=True)
        (work / "sqlite-version-3.54.1/manifest.uuid").write_text("pinned\n")
        (work / "build-clang").mkdir()
        (baseline / "work-path.txt").write_text(str(work))
        output_dir = self.artifacts / "clang"
        output_dir.mkdir()
        report = output_dir / "manifest.json"
        report.write_text('{"status": "PASS"}')
        argv = ["compare_clang.py", "--baseline", str(baseline), "--out",
                str(output_dir), "--clang", sys.executable]
        with mock.patch.object(sys, "argv", argv):
            with self.assertRaisesRegex(ValueError, "already exists"):
                compare_clang.main()
        self.assertFalse(report.exists(), "failure must not leave a stale PASS")

    def test_missing_clang_invalidates_old_report_but_never_the_baseline(self):
        baseline = self.artifacts / "baseline"
        output_dir = self.artifacts / "clang"
        baseline.mkdir()
        output_dir.mkdir()
        source = baseline / "manifest.json"
        report = output_dir / "manifest.json"
        source.write_text('{"status": "PASS"}')
        report.write_text('{"status": "PASS"}')
        argv = ["compare_clang.py", "--baseline", str(baseline), "--out",
                str(output_dir), "--clang", "missing-clang-for-test"]
        with mock.patch.object(sys, "argv", argv), \
             mock.patch.object(compare_clang.shutil, "which", return_value=None):
            with self.assertRaises(SystemExit):
                compare_clang.main()
        self.assertFalse(report.exists(), "a missing tool invalidates old PASS")
        self.assertEqual(source.read_text(), '{"status": "PASS"}')

    def test_failed_paired_rerun_invalidates_old_pass_report(self):
        report = self.artifacts / "paired.json"
        report.write_text('{"status": "PASS"}')
        argv = ["paired_speedtest.py", "--enabled", str(self.artifacts / "enabled"),
                "--disabled", str(self.artifacts / "disabled"), "--out", str(report)]
        with mock.patch.object(sys, "argv", argv), \
             mock.patch.object(paired_speedtest, "input_build", side_effect=ValueError("bad input")):
            with self.assertRaisesRegex(ValueError, "bad input"):
                paired_speedtest.main()
        self.assertFalse(report.exists(), "failure must not leave a stale PASS")

    def test_invalid_paired_configuration_invalidates_old_pass_report(self):
        report = self.artifacts / "paired.json"
        report.write_text('{"status": "PASS"}')
        argv = ["paired_speedtest.py", "--enabled", str(self.artifacts / "enabled"),
                "--disabled", str(self.artifacts / "disabled"), "--repeats", "0",
                "--out", str(report)]
        with mock.patch.object(sys, "argv", argv):
            with self.assertRaises(SystemExit):
                paired_speedtest.main()
        self.assertFalse(report.exists(), "invalid repeats cannot preserve PASS")

    def test_paired_report_cannot_overwrite_input_manifest(self):
        enabled = self.artifacts / "enabled"
        enabled.mkdir()
        manifest = enabled / "manifest.json"
        manifest.write_text('{"status": "PASS"}')
        argv = ["paired_speedtest.py", "--enabled", str(enabled), "--disabled",
                str(self.artifacts / "disabled"), "--out", str(manifest)]
        with mock.patch.object(sys, "argv", argv):
            with self.assertRaises(SystemExit):
                paired_speedtest.main()
        self.assertEqual(manifest.read_text(), '{"status": "PASS"}')

    def test_pair_gate_requires_same_compiler_and_exact_pass_knob(self):
        template = {k: "same" for k in paired_speedtest.IDENTITY_KEYS}
        template.update(speedtest_size=25, speedtest_repeats=3, lccc_disable_passes="")
        template["speedtest1"] = {
            "main": {"verification_hash": "f" * 48, "verified_bytes": 100,
                     "test_count": 1, "runs": {"gcc": [{"tests": [[100, "case"]]}]}}
        }
        enabled, disabled = copy.deepcopy(template), copy.deepcopy(template)
        disabled["lccc_disable_passes"] = "ptr_delta_phi"
        paired_speedtest.identity(enabled, disabled, ["main"])
        for key, value in [("lccc_compiler_sha256", "other"),
                           ("cflags", "-O0"), ("source_zip_sha256", "other")]:
            broken = copy.deepcopy(disabled)
            broken[key] = value
            with self.assertRaisesRegex(ValueError, key):
                paired_speedtest.identity(enabled, broken, ["main"])
        disabled["speedtest1"]["main"]["runs"]["gcc"][0]["tests"] = [[101, "case"]]
        with self.assertRaisesRegex(ValueError, "test identities"):
            paired_speedtest.identity(enabled, disabled, ["main"])
        disabled["lccc_disable_passes"] = "ptr_delta_phi,other"
        with self.assertRaisesRegex(ValueError, "ptr_delta_phi-only disabled"):
            paired_speedtest.identity(enabled, disabled, ["main"])


if __name__ == "__main__":
    unittest.main()
