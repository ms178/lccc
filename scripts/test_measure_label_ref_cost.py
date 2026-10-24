#!/usr/bin/env python3
"""Compiler-free contracts for the paired label-reference cost experiment."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch

import measure_label_ref_cost as measure


class CostExperimentTests(unittest.TestCase):
    def test_source_is_deterministic_and_unsigned(self):
        text = measure.source(2)
        self.assertEqual(text, measure.source(2))
        self.assertEqual(text.count("unsigned branch_"), 2)
        self.assertEqual(text.count("if(x &"), 24)
        self.assertIn("2048u", text)

    def experiment(self, mismatch):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ("before", "after"):
                (root / name).write_text(name)
            argv = ["measure", "--before", str(root / "before"), "--after",
                    str(root / "after"), "--out", str(root / "results"),
                    "--functions", "1", "--reps", "2", "--warmup", "1"]

            def compile_mock(command, **_kwargs):
                arm = Path(command[3]).name
                Path(command[-1]).write_text(arm if mismatch else "same assembly")
                return subprocess.CompletedProcess(command, 0, stdout=b"", stderr=b"")

            with (
                patch("sys.argv", argv),
                patch.object(measure.os, "sched_getaffinity", return_value={0}),
                patch.object(measure.subprocess, "run", side_effect=compile_mock) as run,
                contextlib.redirect_stdout(io.StringIO()),
            ):
                if mismatch:
                    with self.assertRaisesRegex(RuntimeError, "assembly changed"):
                        measure.main()
                else:
                    measure.main()
                    report = json.loads((root / "results/report.json").read_text())
                    samples = report["rows"]["1"]["samples_s"]
                    self.assertEqual({arm: len(values) for arm, values in samples.items()},
                                     {"before": 2, "after": 2})
                    self.assertEqual(run.call_count, 6)

    def test_paired_rounds_exclude_warmup(self):
        self.experiment(False)

    def test_changed_output_is_rejected_not_timed_as_a_win(self):
        self.experiment(True)


if __name__ == "__main__":
    unittest.main()
