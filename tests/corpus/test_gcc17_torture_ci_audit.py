"""Targeted contracts for the GCC 17 lane's diagnostics and inventory."""
from __future__ import annotations

from collections import Counter
from contextlib import redirect_stderr, redirect_stdout
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT))
import torture_ci_summary as summary  # noqa: E402
from tools.corpus.evidence import case_set_digest  # noqa: E402


class PreflightEvidence(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = {**os.environ, "GCC_TORTURE_RESULTS_DIR": str(self.root / "result"),
                    "LCCC_X86_64": "/bin/true", "LCCC_LD": "/bin/true", "GCC_BIN": "/bin/true"}

    def run_driver(self, *args):
        return subprocess.run(["bash", str(ROOT / "scripts/ci_gcc17_torture.sh"), *args],
                              env=self.env, capture_output=True, text=True, timeout=5)

    def expect_error(self, args, text):
        proc = self.run_driver(*args)
        marker = self.root / "result/preflight-failure.txt"
        self.assertEqual(proc.returncode, 2, proc.stderr)
        self.assertIn(text, proc.stderr)
        self.assertIn(text, marker.read_text())

    def test_missing_and_invalid_arch(self):
        self.expect_error([], "usage:")
        self.expect_error(["bogus"], "unsupported architecture")

    def test_missing_and_non_executable_compiler(self):
        path = self.root / "compiler"
        self.env["LCCC_X86_64"] = str(path)
        results = self.root / "result"
        results.mkdir()
        stale = results / "x86_64-execute.json"
        stale.write_text('{"complete": true}')
        self.expect_error(["x86_64"], "not found")
        self.assertFalse(stale.exists(), "old success must not survive a failed retry")
        path.write_text("#!/bin/sh\nexit 0\n")
        path.chmod(0o644)
        self.expect_error(["x86_64"], "not executable")

    def test_missing_linker_and_oracle(self):
        self.env["LCCC_LD"] = str(self.root / "missing-linker")
        self.expect_error(["x86_64"], "standalone linker not found")
        self.env["LCCC_LD"] = "/bin/true"
        self.env["GCC_BIN"] = str(self.root / "missing-gcc")
        self.expect_error(["x86_64"], "reference compiler")

    def test_version_and_jobs(self):
        self.env["GCC_TORTURE_GCC_VERSION"] = "16.2"
        self.expect_error(["x86_64"], "pins GCC 16.2")
        del self.env["GCC_TORTURE_GCC_VERSION"]
        self.env["GCC_TORTURE_JOBS"] = "0"
        self.expect_error(["x86_64"], "invalid GCC_TORTURE_JOBS")


class Inventory(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "report.json"
        self.rows = [
            {"test": "ok.c", "flags": "-O2", "status": "pass"},
            {"test": "bad.c", "flags": "-O2", "status": "compile-fail"},
            {"test": "oracle.c", "flags": "-O2", "status": "reference-fail"},
            {"test": "skip.c", "flags": "-O2", "status": "reference-compile-skip"},
        ]
        self.report = {"schema": 2, "complete": True, "fatal": None, "mode": "compile",
                       "arch": "x86_64", "gcc_checkout_head": "a" * 40,
                       "expected_cases": len(self.rows), "completed_cases": len(self.rows),
                       "flags": ["-O2"], "results": self.rows,
                       "counts": dict(Counter(row["status"] for row in self.rows)),
                       "case_set_sha256": case_set_digest(
                           f"{row['test']}[{row['flags']}]" for row in self.rows)}

    def write(self):
        self.path.write_text(json.dumps(self.report))

    def test_separates_oracle_candidate_and_skips(self):
        self.write()
        result = summary.inventory(self.path)
        self.assertEqual(result["nonpass"], [["bad.c", "-O2", "compile-fail"],
                                             ["oracle.c", "-O2", "reference-fail"],
                                             ["skip.c", "-O2", "reference-compile-skip"]])
        self.assertEqual(result["counts"]["pass"], 1)

    def test_refuses_partial_duplicate_unknown_and_legacy(self):
        for mutation in (lambda d: d.update(complete=False),
                         lambda d: d["results"].append(d["results"][0]),
                         lambda d: d["results"][0].update(status="new-status"),
                         lambda d: d.update(schema=1)):
            saved = json.loads(json.dumps(self.report))
            mutation(self.report)
            self.write()
            with self.assertRaises(ValueError):
                summary.inventory(self.path)
            self.report = saved

    def test_missing_report_is_not_a_pass(self):
        with redirect_stderr(io.StringIO()), redirect_stdout(io.StringIO()):
            self.assertEqual(summary.main([str(self.path)]), 2)

    def test_annotation_escapes_workflow_metacharacters(self):
        self.rows[1]["test"] = "a%\n::warning::.c"
        self.report["case_set_sha256"] = case_set_digest(
            f"{row['test']}[{row['flags']}]" for row in self.rows)
        self.write()
        output = io.StringIO()
        with redirect_stdout(output):
            self.assertEqual(summary.main([str(self.path)]), 0)
        self.assertEqual(len(output.getvalue().splitlines()), 1)
        self.assertIn("%25", output.getvalue())


if __name__ == "__main__":
    unittest.main()
