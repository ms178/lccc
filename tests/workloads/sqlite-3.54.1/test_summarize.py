#!/usr/bin/env python3
"""Pure-logic tests for the SQLite speedtest1 result/identity gate."""
import copy
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

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
