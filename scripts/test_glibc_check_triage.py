#!/usr/bin/env python3
"""Unit tests for scripts/glibc_check_triage.py (BACKLOG MS-11)."""
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import glibc_check_triage as t  # noqa: E402


class ClassifyTests(unittest.TestCase):
    def test_environment_outranks_crash(self):
        log = "cannot open shared object file\nDidn't expect signal from child: got `Segmentation fault'"
        self.assertEqual(t.classify("FAIL", log)[0], "environment")

    def test_unsupported_outranks_crash(self):
        log = "ifunc unsupported\nDidn't expect signal from child: got `Segmentation fault'"
        self.assertEqual(t.classify("FAIL", log)[0], "unsupported-feature")

    def test_signal_is_miscompile(self):
        for sig in ("Segmentation fault", "Illegal instruction", "Bus error", "Aborted"):
            log = f"Didn't expect signal from child: got `{sig}'"
            self.assertEqual(t.classify("FAIL", log)[0], "miscompile", sig)

    def test_check_h_mismatch_is_miscompile(self):
        log = "error: tst-x.c:42: (a) != (b)\n"
        # no "got" text: support/check.h style mismatches
        self.assertEqual(t.classify("FAIL", "error: tst-x.c:42: value mismatch\n")[0], "miscompile")
        self.assertEqual(t.classify("FAIL", log)[0], "miscompile")
        self.assertEqual(t.classify("FAIL", "error: something else happened\n")[0], "unclassified")

    def test_passing_buckets(self):
        self.assertEqual(t.classify("PASS", "")[0], "pass")
        self.assertEqual(t.classify("XFAIL", "")[0], "xfail")
        self.assertEqual(t.classify("UNSUPPORTED", "")[0], "unsupported-feature")

    def test_last_result_wins(self):
        self.assertEqual(t.parse_results("FAIL: a/b\nPASS: a/b\n"), {"a/b": "PASS"})


class CliTests(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, str(HERE / "glibc_check_triage.py"), *args],
                              capture_output=True, text=True)

    def test_self_test(self):
        self.assertEqual(self.run_cli("--self-test").returncode, 0)

    def test_exit_codes_and_reports(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "string").mkdir()
            (root / "tests.sum").write_text("FAIL: string/tst-a\nPASS: string/tst-b\n")
            (root / "string/tst-a.out").write_text(
                "Didn't expect signal from child: got `Segmentation fault'\n")
            out = root / "r.json"
            res = self.run_cli(str(root), "--json", str(out), "--markdown", str(root / "r.md"))
            self.assertEqual(res.returncode, 1, res.stderr)
            data = json.loads(out.read_text())
            self.assertEqual(data["summary"]["miscompiles"], ["string/tst-a"])
            self.assertEqual(self.run_cli(str(root), "--allow-miscompiles", "1").returncode, 0)
            self.assertEqual(self.run_cli(str(root / "nope")).returncode, 2)
            (root / "empty").mkdir()
            self.assertEqual(self.run_cli(str(root / "empty")).returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=1)
