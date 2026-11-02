#!/usr/bin/env python3
"""Hermetic oracle coverage/ISA controls: no network or compiler required."""
import argparse
import contextlib
import io
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import benchmark_codegen as benchmark
import loop_density_oracle as density
import tight_loop_oracle as tight
import tune_oracle as tune


class SurveyContracts(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / "probe.c"
        self.source.write_text("int probe(int x) { return x + 1; }\n")
        self.report = self.root / "report.json"
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
        stack.enter_context(contextlib.redirect_stderr(io.StringIO()))

    def tight_run(self, remote=None, local=None, program="probe"):
        response = {"asm": [{"text": "probe:"}, {"text": "ret"}]}
        with patch.object(tight, "PROGRAMS", self.root), \
             patch.object(tight.subprocess, "check_output", return_value=str(self.root)), \
             patch.object(tight.gb, "compile_on_godbolt", side_effect=remote, return_value=response), \
             patch.object(tight, "lccc_decisions", return_value=local or ({".L1": 4}, {}, None)) as compile_local:
            status = tight.main(["cc", "--march", "x86-64-v3", "--oracles", "vendor",
                                 "--json", str(self.report), program])
        return status, compile_local

    def test_alignment_group_selects_strongest_unconditional_in_either_order(self):
        for directives in ([".p2align 5", ".p2align 3"], [".p2align 3", ".p2align 5"]):
            lines = ["probe:", *directives, ".L1:", "dec %eax", "jne .L1"]
            self.assertEqual(tight.oracle_headers(lines)[".L1"][1], 5)
            self.assertEqual(tight.directive_groups(lines)[".L1"], 5)

    def test_bounded_alignment_is_not_an_unconditional_guarantee(self):
        lines = ["probe:", ".p2align 3", ".p2align 6,,2", ".L1:", "dec %eax", "jne .L1"]
        self.assertEqual(tight.oracle_headers(lines)[".L1"][1], 3)
        self.assertEqual(tight.directive_groups(lines)[".L1"], 3)

    def test_explicit_fill_does_not_hide_max_skip(self):
        lines = ["probe:", ".p2align 5,0x90,2", ".L1:", "dec %eax", "jne .L1"]
        self.assertEqual(tight.oracle_headers(lines)[".L1"][1], "cascade")
        self.assertEqual(tight.directive_groups(lines)[".L1"], "cascade")

    def test_byte_emitting_directive_breaks_alignment_group(self):
        lines = ["probe:", ".p2align 5", ".byte 0x90", ".L1:", "dec %eax", "jne .L1"]
        self.assertIsNone(tight.oracle_headers(lines)[".L1"][1])
        self.assertIsNone(tight.directive_groups(lines)[".L1"])

    def test_comments_do_not_create_alignment_directives(self):
        lines = ["probe:", "# .p2align 5", ".L1:", "dec %eax", "jne .L1"]
        self.assertIsNone(tight.oracle_headers(lines)[".L1"][1])
        self.assertIsNone(tight.directive_groups(lines)[".L1"])

    def test_census_bucket_names_do_not_invent_exponents(self):
        self.assertEqual(tight.bucket_name(4), "2^4")
        self.assertEqual(tight.bucket_name("cascade"), "cascade")
        self.assertEqual(tight.bucket_name("unaligned"), "unaligned")

    def test_vendor_unaligned_is_distinct_from_bounded_alignment(self):
        lines = ["probe:", ".L1:", "dec %eax", "jne .L1"]
        self.assertIsNone(tight.oracle_headers(lines)[".L1"][1])
        bounded = ["probe:", ".p2align 4,,2", *lines[1:]]
        self.assertEqual(tight.oracle_headers(bounded)[".L1"][1], "cascade")

    def test_tight_matched_isa_and_header_provider(self):
        status, local = self.tight_run()
        self.assertEqual(status, 0)
        self.assertEqual(local.call_args.args[1], "-O2 -march=x86-64-v3")
        self.assertEqual(local.call_args.args[3], str(self.root))
        self.assertTrue(json.loads(self.report.read_text())["complete"])

    def test_tight_remote_failure_is_not_success(self):
        status, _ = self.tight_run(remote=tight.gb.GodboltError("offline"))
        self.assertEqual(status, 1)
        self.assertFalse(json.loads(self.report.read_text())["complete"])

    def test_tight_local_failure_is_not_success(self):
        status, _ = self.tight_run(local=(None, None, "compiler failed"))
        self.assertEqual(status, 1)
        self.assertEqual(json.loads(self.report.read_text())["errors"][0]["compiler"], "lccc")

    def test_tight_failure_without_stderr_keeps_an_incomplete_receipt(self):
        status, _ = self.tight_run(local=(None, None, ""))
        self.assertEqual(status, 1)
        self.assertFalse(json.loads(self.report.read_text())["complete"])

    def test_tight_cli_preserves_unaligned_vendor_category(self):
        response = {"asm": [{"text": s} for s in ["probe:", ".L1:", "dec %eax", "jne .L1"]]}
        status, _ = self.tight_run(remote=lambda *args, **kwargs: response)
        self.assertEqual(status, 0)
        histogram = json.loads(self.report.read_text())["rows"][0]["oracle_histograms"]["vendor"]
        self.assertEqual(histogram, {"unaligned": 1})

    def test_tight_missing_source_is_not_empty_success(self):
        status, _ = self.tight_run(program="missing")
        self.assertEqual(status, 1)

    def test_tight_mixed_cascade_histogram_is_printable(self):
        status, _ = self.tight_run(local=({".L1": 4, ".L2": "cascade"}, {}, None))
        self.assertEqual(status, 0)

    def test_tight_passes_same_flags_to_text_and_integrated_modes(self):
        calls = []
        def run(command, **kwargs):
            calls.append(command)
            if "-S" in command:
                Path(command[command.index("-o") + 1]).write_text("probe:\n.L1:\ndec %eax\njne .L1\nret\n")
            self.assertEqual(kwargs["timeout"], 60)
            return subprocess.CompletedProcess(command, 0, "", "")
        with patch.object(tight.subprocess, "run", side_effect=run):
            effective, _, _ = tight.lccc_decisions("cc", "-O2 -march=x86-64-v3 -fwrapv", self.source, str(self.root))
        self.assertEqual(effective[".L1"], "unaligned")
        for command in calls:
            for flag in shlex.split("-O2 -march=x86-64-v3 -fwrapv"):
                self.assertIn(flag, command)

    def density_run(self, remote=None, local=None):
        with patch.object(density.oracle, "_local_compile", side_effect=local, return_value=["probe:", "ret"]), \
             patch.object(density.oracle, "_compile_remote", side_effect=remote, return_value=["probe:", "ret"]), \
             patch.object(density, "measure_all", return_value={}):
            return density.main([str(self.source), "--oracles", "vendor", "--json", str(self.report)])

    def test_density_remote_failure_marks_incomplete(self):
        self.assertEqual(self.density_run(remote=RuntimeError("offline")), 1)
        self.assertFalse(json.loads(self.report.read_text())["complete"])

    def test_density_local_failure_marks_incomplete(self):
        self.assertEqual(self.density_run(local=RuntimeError("bad local compiler")), 1)
        self.assertFalse(json.loads(self.report.read_text())["complete"])

    def test_density_partial_vendor_set_is_never_ranked(self):
        metrics = {"probe": {"insns": 4, "bytes_per_trip": 8, "insns_per_byte": 0.5,
                             "widest_vector_bits": 0, "loop_label": ".L1"}}
        with patch.object(density.oracle, "_local_compile", return_value=["probe:", "ret"]), \
             patch.object(density.oracle, "_compile_remote", side_effect=[["probe:", "ret"], RuntimeError("offline")]), \
             patch.object(density, "measure_all", return_value=metrics):
            status = density.main([str(self.source), "--oracles", "one,two", "--json", str(self.report)])
        report = json.loads(self.report.read_text())
        self.assertEqual(status, 1)
        self.assertEqual(report["ranked"], [])
        self.assertIn("incomplete compiler coverage", report["unranked"][0]["note"])

    def test_density_no_measurable_loop_is_not_an_invented_win(self):
        self.assertEqual(self.density_run(), 0)
        report = json.loads(self.report.read_text())
        self.assertTrue(report["complete"])
        self.assertEqual(report["ranked"], [])

    def tune_run(self, remote=None, function=None):
        args = argparse.Namespace(probe=str(self.source), lccc="cc", flags="-O2", oracles="vendor",
                                  function=function, save=None)
        with patch.object(tune, "lccc_compile", return_value="probe:\n ret\n"), \
             patch.object(tune, "godbolt_compile", side_effect=remote, return_value="probe:\n ret\n"):
            return tune.cmd_compare(args)

    def test_tune_remote_failure_is_not_success(self):
        self.assertEqual(self.tune_run(remote=RuntimeError("offline")), 1)

    def test_tune_remote_failure_still_reports_the_local_table(self):
        # Failing closed must not discard the measurement that did succeed.
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(self.tune_run(remote=RuntimeError("offline")), 1)
        self.assertIn("`probe`", out.getvalue())

    def test_tune_missing_requested_function_is_not_success(self):
        self.assertEqual(self.tune_run(function="missing"), 1)

    def test_tune_positive_control(self):
        self.assertEqual(self.tune_run(function="probe"), 0)

    def test_empty_header_provider_is_not_current_directory(self):
        with patch.object(benchmark.subprocess, "check_output", return_value=""):
            with self.assertRaises(SystemExit) as error:
                benchmark.main(["--out", str(self.root / "out")])
        self.assertEqual(error.exception.code, 2)
        self.assertFalse((self.root / "out").exists())

    def test_duplicate_registry_ids_are_rejected(self):
        first = benchmark.bench.BENCHMARKS[0]
        with patch.object(benchmark.bench, "BENCHMARKS", [first, first]):
            with self.assertRaises(SystemExit) as error:
                benchmark.main(["--out", str(self.root / "out")])
        self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
