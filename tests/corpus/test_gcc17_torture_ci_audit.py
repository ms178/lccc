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
import measure_decl_scaling as scaling  # noqa: E402
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
        path.unlink()
        path.mkdir()
        self.expect_error(["x86_64"], "not a regular file")

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

    def test_refuses_malformed_rows_without_crashing_and_missing_identity(self):
        for mutation in (lambda d: d["results"][0].pop("status"),
                         lambda d: d["results"][0].update(status=[]),
                         lambda d: d["results"][0].update(flags=[]),
                         lambda d: d.update(arch=None),
                         lambda d: d.update(gcc_checkout_head="unknown"),
                         lambda d: d.pop("counts")):
            saved = json.loads(json.dumps(self.report))
            mutation(self.report)
            self.write()
            with self.assertRaises(ValueError):
                summary.inventory(self.path)
            self.report = saved

    def test_missing_report_is_not_a_pass(self):
        with redirect_stderr(io.StringIO()), redirect_stdout(io.StringIO()):
            self.assertEqual(summary.main([str(self.path)]), 2)

    def test_oversized_inventory_is_valid_json_and_reports_omissions(self):
        self.write()
        result = summary.inventory(self.path)
        result["nonpass"] = [[f"case-{i}.c", "-O2", "compile-fail"] for i in range(1000)]
        payload = summary.annotation_payload(result)
        parsed = json.loads(payload)
        self.assertLessEqual(len(payload.encode()), 3800)
        self.assertGreater(parsed["nonpass_omitted"], 0)
        self.assertEqual(len(parsed["nonpass"]) + parsed["nonpass_omitted"], 1000)

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


class ScalingProbe(unittest.TestCase):
    def test_generated_declarations_are_distinct_and_well_scoped(self):
        block = scaling.source("block", 3)
        self.assertEqual(block, "void f(void) {\nint v0;\nint v1;\nint v2;\n}\n")
        self.assertEqual(scaling.source("external", 2), "extern int v0;\nextern int v1;\n")
        with self.assertRaises(ValueError):
            scaling.source("unknown", 1)

    def test_success_records_both_shapes_and_alternates_order(self):
        with tempfile.TemporaryDirectory() as td:
            compiler = Path(td) / "compiler"
            compiler.write_text("#!/usr/bin/env python3\n"
                                "import os, pathlib, sys\n"
                                "pathlib.Path(sys.argv[-1]).write_bytes(bytes.fromhex('7f454c46020101' + '00' * 9 + '01003e00'))\n"
                                "if os.getenv('CCC_TIME_PHASES'): print('[TIME] sema: 0.125s', file=sys.stderr)\n")
            compiler.chmod(0o755)
            out = Path(td) / "report.json"
            proc = subprocess.run(
                [sys.executable, str(ROOT / "scripts/measure_decl_scaling.py"),
                 "--lccc", str(compiler), "--gcc", str(compiler), "--sizes", "1",
                 "--rounds", "2", "--json", str(out)], capture_output=True, text=True, timeout=5)
            self.assertEqual(proc.returncode, 0, proc.stderr)
            report = json.loads(out.read_text())
            self.assertEqual([r["compiler"] for r in report["results"]],
                             ["lccc", "gcc", "gcc", "lccc"] * 2)
            self.assertEqual([r["shape"] for r in report["results"]],
                             ["block"] * 4 + ["external"] * 4)
            self.assertEqual([r["sema_s"] for r in report["results"] if r["compiler"] == "lccc"],
                             [0.125] * 4)
            self.assertTrue(all(r["returncode"] == 0 and len(r["input_sha256"]) == 64
                                and len(r["object_sha256"]) == 64
                                and r["command"][-2] == "-o" for r in report["results"]))
            prefix = "::notice title=Declaration scaling measurement::"
            self.assertTrue(proc.stdout.startswith(prefix))
            notice = json.loads(proc.stdout[len(prefix):])
            self.assertEqual(notice["total_samples"], 8)
            self.assertEqual(notice["omitted_samples"], 0)
            self.assertEqual(len(notice["samples"]), 8)

    def test_large_measurement_annotation_remains_complete_json(self):
        row = {"shape": "block", "n": 40000, "round": 0, "compiler": "lccc",
               "wall_s": 1.0, "sema_s": 0.5}
        output = {"compilers": {"lccc": {"sha256": "a" * 64}, "gcc": {"sha256": "b" * 64}},
                  "flags": ["-O0"], "results": [row] * 1000}
        payload = scaling.annotation_payload(output)
        parsed = json.loads(payload)
        self.assertLessEqual(len(payload.encode()), 3800)
        self.assertGreater(parsed["omitted_samples"], 0)
        self.assertEqual(parsed["total_samples"], 1000)
        self.assertEqual(len(parsed["samples"]) + parsed["omitted_samples"], 1000)

    def test_failed_compiler_cannot_publish_a_measurement(self):
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "report.json"
            proc = subprocess.run(
                [sys.executable, str(ROOT / "scripts/measure_decl_scaling.py"),
                 "--lccc", "/bin/false", "--gcc", "/bin/true", "--sizes", "1",
                 "--rounds", "1", "--json", str(out)], capture_output=True, text=True, timeout=5)
            self.assertEqual(proc.returncode, 1)
            self.assertIn("exited", proc.stderr)
            self.assertFalse(out.exists())

    def test_success_exit_without_object_removes_stale_report(self):
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "report.json"
            out.write_text('{"old": true}')
            proc = subprocess.run(
                [sys.executable, str(ROOT / "scripts/measure_decl_scaling.py"),
                 "--lccc", "/bin/true", "--gcc", "/bin/true", "--sizes", "1",
                 "--rounds", "1", "--json", str(out)], capture_output=True, text=True, timeout=5)
            self.assertEqual(proc.returncode, 1)
            self.assertIn("did not produce an object", proc.stderr)
            self.assertFalse(out.exists())

    def test_wrong_object_header_is_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            compiler = Path(td) / "compiler"
            compiler.write_text("#!/usr/bin/env python3\n"
                                "import pathlib, sys\n"
                                "pathlib.Path(sys.argv[-1]).write_bytes(b'not an ELF object')\n")
            compiler.chmod(0o755)
            out = Path(td) / "report.json"
            proc = subprocess.run(
                [sys.executable, str(ROOT / "scripts/measure_decl_scaling.py"),
                 "--lccc", str(compiler), "--gcc", str(compiler), "--sizes", "1",
                 "--rounds", "1", "--json", str(out)], capture_output=True, text=True, timeout=5)
            self.assertEqual(proc.returncode, 1)
            self.assertIn("did not produce x86-64 ELF relocatable", proc.stderr)
            self.assertFalse(out.exists())

    def test_report_must_not_overwrite_compiler(self):
        with tempfile.TemporaryDirectory() as td:
            compiler = Path(td) / "compiler"
            compiler.write_text("compiler contents")
            out = Path(td) / "report.json"
            os.link(compiler, out)
            proc = subprocess.run(
                [sys.executable, str(ROOT / "scripts/measure_decl_scaling.py"),
                 "--lccc", str(compiler), "--gcc", "/bin/true", "--sizes", "1",
                 "--rounds", "1", "--json", str(out)], capture_output=True, text=True, timeout=5)
            self.assertEqual(proc.returncode, 1)
            self.assertIn("report path names a compiler binary", proc.stderr)
            self.assertEqual(compiler.read_text(), "compiler contents")

    def test_missing_phase_timer_is_not_a_valid_measurement(self):
        with tempfile.TemporaryDirectory() as td:
            out = Path(td) / "report.json"
            compiler = Path(td) / "compiler"
            compiler.write_text("#!/usr/bin/env python3\n"
                                "import pathlib, sys\n"
                                "pathlib.Path(sys.argv[-1]).write_bytes(bytes.fromhex('7f454c46020101' + '00' * 9 + '01003e00'))\n")
            compiler.chmod(0o755)
            proc = subprocess.run(
                [sys.executable, str(ROOT / "scripts/measure_decl_scaling.py"),
                 "--lccc", str(compiler), "--gcc", str(compiler), "--sizes", "1",
                 "--rounds", "1", "--json", str(out)], capture_output=True, text=True, timeout=5)
            self.assertEqual(proc.returncode, 1)
            self.assertIn("phase timer missing", proc.stderr)
            self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
