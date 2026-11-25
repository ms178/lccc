#!/usr/bin/env python3
"""Pure-logic tests for the SQLite speedtest1 result/identity gate."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
