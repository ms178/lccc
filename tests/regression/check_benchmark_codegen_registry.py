#!/usr/bin/env python3
"""Offline, mock-only tests of per-workload codegen request/evidence identity."""
from __future__ import annotations

import contextlib
import io
import json
import os
import subprocess
import textwrap
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import benchmark_codegen as tool


class RegistryTests(unittest.TestCase):
    def invoke(self, directory, oracle, only="linux_find_bit,linux_find_bit_scaled,lz4_compress,lz4_match_extend"):
        with patch.object(tool.subprocess, "check_output", side_effect=[str(directory), "GCC mock identity"]), \
                patch.object(tool.codegen_oracle, "main", side_effect=oracle), \
                contextlib.redirect_stdout(io.StringIO()):
            return tool.main(["--local", "mock-lccc", "--gcc", "mock-gcc", "--oracles", "",
                              "--only", only, "--out", str(directory)])

    @staticmethod
    def report(command):
        destination = Path(command[command.index("--json") + 1])
        destination.write_text(json.dumps([{
            "source": command[0], "errors": [],
            "records": [{"key": "lccc", "flags": command[command.index("--local-flags") + 1]}],
        }]))
        return 0

    def test_shared_sources_are_distinct_compilation_requests(self):
        commands = []

        def oracle(command):
            commands.append(command)
            return self.report(command)

        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.assertEqual(self.invoke(directory, oracle), 0)
            index = json.loads((directory / "index.json").read_text())
            self.assertTrue(index["complete"])
            self.assertEqual(len(index["results"]), 4)
            self.assertFalse(index["failures"])
            flags = {Path(c[c.index("--json") + 1]).parent.name: c[c.index("--flags") + 1] for c in commands}
            self.assertNotIn("-DPASSES=65536", flags["linux_find_bit"])
            self.assertIn("-DPASSES=65536", flags["linux_find_bit_scaled"])
            self.assertNotIn("-DMATCH_RICH=1", flags["lz4_compress"])
            self.assertIn("-DMATCH_RICH=1", flags["lz4_match_extend"])
            for command in commands:
                self.assertIn("-fwrapv", command[command.index("--flags") + 1])
                self.assertNotIn("-I", command[command.index("--flags") + 1])
                self.assertIn("-I", command[command.index("--local-flags") + 1])
                self.assertNotIn("--rank", command)

    def test_stale_report_cannot_rescue_missing_output(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            stale = directory / "linux_find_bit" / "result.json"
            stale.parent.mkdir()
            stale.write_text('[{"records": [{"key": "lccc"}]}]')
            self.assertEqual(self.invoke(directory, lambda _: 0, only="linux_find_bit"), 1)
            self.assertFalse(stale.exists())
            index = json.loads((directory / "index.json").read_text())
            self.assertEqual(len(index["failures"]), 1)

    def test_oracle_error_is_not_a_successful_survey(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.assertEqual(self.invoke(directory, lambda _: 1, only="linux_find_bit"), 1)

    def test_workflow_annotations_fit_observed_api_limits(self):
        workflow = (ROOT / ".github/workflows/debian-oracle.yml").read_text()
        code = textwrap.dedent(workflow.split("python3 - <<'PY'\n", 1)[1].split("\n          PY", 1)[0])
        rows = []
        for item in tool.bench.BENCHMARKS:
            rows.append({
                "name": item.name,
                "correctness": {key: {"matches_baseline": True, "output": "large-output" * 1000}
                                for key in ("lccc", "gcc")},
                "compile": {key: {"ok": True, "source_sha256": "a" * 64,
                                  "command": ["compiler", "long-argument" * 1000]}
                            for key in ("lccc", "gcc")},
            })
        data = {"benchmarks": rows, "settings": {"skip_perf": True},
                "metadata": {"compiler_versions": {"gcc": "GCC 16.2.0", "lccc": "LCCC"},
                             "git": {"revision": "b" * 40}, "meminfo": "huge" * 10000}}
        with tempfile.TemporaryDirectory() as name:
            (Path(name) / "debian-results.json").write_text(json.dumps(data))
            result = subprocess.run([sys.executable, "-c", code], cwd=name,
                                    env={**os.environ, "PYTHONPATH": str(ROOT)},
                                    capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        notices = result.stdout.splitlines()
        self.assertLessEqual(len(notices), 10)
        decoded = []
        for line in notices:
            payload = line.split("::", 2)[2].replace("%0D", "\r").replace("%0A", "\n").replace("%25", "%")
            self.assertLessEqual(len(payload.encode()), 3900)
            decoded.append(json.loads(payload))
        self.assertEqual(decoded[0]["workloads"], len(rows))
        published = [row["name"] for group in decoded[1:] for row in group]
        self.assertCountEqual(published, [row["name"] for row in rows])

    def test_profile_failure_cannot_launch_or_certify_the_survey(self):
        profile = {"complete": False, "target": "Haswell", "compilers": {
            "icc": {"id": "cicc2021100", "ok": False, "error": "option ignored"}}}
        with tempfile.TemporaryDirectory() as name, \
                patch.object(tool.subprocess, "check_output", side_effect=[name, "GCC mock"]), \
                patch.object(tool.oracle_profile, "verify", return_value=profile), \
                patch.object(tool.codegen_oracle, "main") as survey, \
                contextlib.redirect_stdout(io.StringIO()):
            rc = tool.main(["--gcc", "mock", "--local", "mock", "--only", "nbody",
                            "--flags=-O2 -march=haswell -fwrapv", "--verify-haswell-profile",
                            "--oracles", "icc", "--out", name])
            self.assertEqual(rc, 1)
            survey.assert_not_called()
            index = json.loads((Path(name) / "index.json").read_text())
            self.assertFalse(index["complete"])
            self.assertFalse(index["target_profile"]["complete"])
            self.assertTrue(index["failures"])
            self.assertEqual(index["results"], {})
            self.assertTrue((Path(name) / "profile.json").is_file())

    def test_wrong_source_or_flags_fail_identity_validation(self):
        for mismatch in ("source", "flags"):
            def oracle(command):
                self.report(command)
                path = Path(command[command.index("--json") + 1])
                data = json.loads(path.read_text())
                if mismatch == "source":
                    data[0]["source"] = "another-source.c"
                else:
                    data[0]["records"][0]["flags"] = "-O0"
                path.write_text(json.dumps(data))
                return 0
            with self.subTest(mismatch=mismatch), tempfile.TemporaryDirectory() as name:
                self.assertEqual(self.invoke(Path(name), oracle, only="linux_find_bit"), 1)


if __name__ == "__main__":
    unittest.main()
