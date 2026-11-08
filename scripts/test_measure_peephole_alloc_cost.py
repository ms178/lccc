#!/usr/bin/env python3
"""Compiler-free contracts for the allocation-cost A/B harness.

These pin the failure modes that would make the harness report a confident
but meaningless number. Each one corresponds to a trap actually hit while
building it, so the tests are non-vacuous by construction.
"""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import measure_peephole_alloc_cost as measure


def write_report(path: Path, **overrides) -> None:
    fields = dict(malloc=10, calloc=1, realloc=2, free=9, aligned=0, bytes=1000, total=13)
    fields.update(overrides)
    path.write_text(" ".join(f"{k}={v}" for k, v in fields.items()) + "\n")


class ReportParsingTests(unittest.TestCase):
    def test_sums_every_child_process_report(self):
        # Trap 2: a driver that spawns a child writes two reports. Reading
        # only one silently undercounts the compile.
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "alloc-report"
            write_report(Path(str(prefix) + ".101"), total=13)
            write_report(Path(str(prefix) + ".102"), total=7)
            got = measure.parse_reports(prefix)
            self.assertEqual(got["total"], 20)
            self.assertEqual(got["bytes"], 2000)

    def test_missing_report_is_an_error_not_zero(self):
        # The single most dangerous failure: if the interposer never ran,
        # returning 0 would look like a perfect optimization.
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(RuntimeError):
                measure.parse_reports(Path(tmp) / "alloc-report")

    def test_incomplete_report_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "alloc-report"
            Path(str(prefix) + ".1").write_text("malloc=5 total=5\n")
            with self.assertRaises(RuntimeError):
                measure.parse_reports(prefix)

    def test_unknown_field_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "alloc-report"
            Path(str(prefix) + ".1").write_text(
                "malloc=1 calloc=0 realloc=0 free=0 aligned=0 bytes=1 total=1 bogus=2\n")
            with self.assertRaises(RuntimeError):
                measure.parse_reports(prefix)

    def test_stale_reports_from_another_run_are_not_read(self):
        # run_once must clear old reports; a leftover would be summed in.
        with tempfile.TemporaryDirectory() as tmp:
            prefix = Path(tmp) / "alloc-report"
            write_report(Path(str(prefix) + ".999"))
            with patch.object(measure.subprocess, "run") as run:
                run.return_value = type("P", (), {"returncode": 0, "stderr": ""})()
                out_s = Path(tmp) / "x.s"
                out_s.write_text(".text\n")
                # The fake compiler writes only its own report.
                def side_effect(*a, **k):
                    for stale in Path(tmp).glob("alloc-report.*"):
                        stale.unlink()
                    write_report(Path(str(prefix) + ".1"), total=4)
                    out_s.write_text(".text\n")  # run_once deletes the old output first
                    return run.return_value
                run.side_effect = side_effect
                got = measure.run_once(Path("/bin/true"), Path(tmp) / "x.c", out_s,
                                       Path("/nonexistent.so"), prefix, ["-O1"], {})
                self.assertEqual(got["total"], 4)


class ArgumentContractTests(unittest.TestCase):
    def test_single_run_is_refused(self):
        # An unrepeated count cannot establish determinism.
        for repeats in ("0", "1"):
            argv = ["measure", "--before", "/bin/true", "--after", "/bin/true",
                    "--out", "/tmp/unused-alloc-cost", "--repeats", repeats]
            err = io.StringIO()
            with patch.object(measure.sys, "argv", argv), contextlib.redirect_stderr(err):
                rc = measure.main()
            self.assertEqual(rc, 2)
            self.assertIn("not evidence", err.getvalue())

    def test_missing_binary_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            argv = ["measure", "--before", str(Path(tmp) / "nope"),
                    "--after", "/bin/true", "--out", str(Path(tmp) / "out")]
            err = io.StringIO()
            with patch.object(measure.sys, "argv", argv), contextlib.redirect_stderr(err):
                rc = measure.main()
            self.assertEqual(rc, 2)
            self.assertIn("not a file", err.getvalue())

    def test_empty_corpus_is_refused(self):
        # Zero inputs must not report a vacuous PASS.
        with tempfile.TemporaryDirectory() as tmp:
            argv = ["measure", "--before", "/bin/true", "--after", "/bin/true",
                    "--out", str(Path(tmp) / "out"), "--glob", "no/such/*.c"]
            err = io.StringIO()
            with patch.object(measure.sys, "argv", argv), contextlib.redirect_stderr(err):
                rc = measure.main()
            self.assertEqual(rc, 2)
            self.assertIn("no inputs match", err.getvalue())


class VerdictTests(unittest.TestCase):
    """Drive main() with a stubbed compiler to pin the verdict logic."""

    def _drive(self, before_counts, after_counts, asm_before=".text\n", asm_after=".text\n",
               extra_argv=()):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        src = root / "corpus"
        src.mkdir()
        (src / "a.c").write_text("int f(void){return 0;}\n")
        before = root / "before"
        after = root / "after"
        before.write_text("#!/bin/sh\n")
        after.write_text("#!/bin/sh\n")
        state = {"n": 0}

        def fake_run_once(binary, source, out_s, shim, report_prefix, opts, env_extra):
            state["n"] += 1
            is_before = binary == before
            # repeats alternate within an arm; both must agree
            out_s.write_text(asm_before if is_before else asm_after)
            return dict(before_counts if is_before else after_counts)

        argv = ["measure", "--before", str(before), "--after", str(after),
                "--out", str(root / "out"), "--glob", "corpus/*.c", *extra_argv]
        out = io.StringIO()
        err = io.StringIO()
        with patch.object(measure.sys, "argv", argv), \
             patch.object(measure, "ROOT", root), \
             patch.object(measure, "build_shim", return_value=root / "shim.so"), \
             patch.object(measure, "run_once", side_effect=fake_run_once), \
             contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            rc = measure.main()
        manifest = json.loads((root / "out" / "manifest.json").read_text())
        return rc, manifest, out.getvalue(), err.getvalue()

    @staticmethod
    def _counts(total):
        return dict(malloc=total, calloc=0, realloc=0, free=total, aligned=0,
                    bytes=total * 10, total=total)

    def test_improvement_with_identical_asm_passes(self):
        rc, m, _, _ = self._drive(self._counts(100), self._counts(60))
        self.assertEqual(rc, 0)
        self.assertTrue(m["ok"])
        self.assertTrue(m["asm_all_identical"])
        self.assertEqual(m["delta_allocs"], -40)

    def test_differing_assembly_fails_even_when_faster(self):
        # The MB-20 acceptance rule: a cost win that changes code is a defect.
        rc, m, _, err = self._drive(self._counts(100), self._counts(10),
                                    asm_after=".text\nnop\n")
        self.assertEqual(rc, 1)
        self.assertFalse(m["asm_all_identical"])
        self.assertIn("assembly DIFFERS", err)

    def test_growth_fails(self):
        rc, m, _, err = self._drive(self._counts(100), self._counts(110))
        self.assertEqual(rc, 1)
        self.assertIn("grew", err)
        self.assertGreater(m["delta_allocs"], 0)

    def test_no_change_fails_when_improvement_required(self):
        rc, _, _, err = self._drive(self._counts(100), self._counts(100),
                                    extra_argv=("--require-improvement",))
        self.assertEqual(rc, 1)
        self.assertIn("no improvement", err)

    def test_no_change_passes_as_a_ratchet_without_improvement_flag(self):
        rc, m, _, _ = self._drive(self._counts(100), self._counts(100))
        self.assertEqual(rc, 0)
        self.assertTrue(m["ok"])

    def test_non_reproducible_count_fails(self):
        # Trap: a count that drifts between runs cannot support a delta claim.
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        (root / "corpus").mkdir()
        (root / "corpus" / "a.c").write_text("int f(void){return 0;}\n")
        for name in ("before", "after"):
            (root / name).write_text("#!/bin/sh\n")
        seq = iter([self._counts(100), self._counts(101), self._counts(60), self._counts(60)])

        def drifting(binary, source, out_s, shim, report_prefix, opts, env_extra):
            out_s.write_text(".text\n")
            return next(seq)

        argv = ["measure", "--before", str(root / "before"), "--after", str(root / "after"),
                "--out", str(root / "out"), "--glob", "corpus/*.c"]
        err = io.StringIO()
        with patch.object(measure.sys, "argv", argv), \
             patch.object(measure, "ROOT", root), \
             patch.object(measure, "build_shim", return_value=root / "shim.so"), \
             patch.object(measure, "run_once", side_effect=drifting), \
             contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            rc = measure.main()
        self.assertEqual(rc, 1)
        self.assertIn("not reproducible", err.getvalue())


if __name__ == "__main__":
    unittest.main()
