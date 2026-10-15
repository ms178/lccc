"""Compiler-work measurement verdicts without a compiler or Valgrind process."""
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import callgrind_compiler as profile


class CompilerWorkVerdicts(unittest.TestCase):
    def measure(self, failure=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "case.c"
            source.write_text("int f(void) { return 1; }\n")
            compiler = root / "compiler"
            compiler.write_text("fake executable: never executed")
            output = root / "profiles"

            def run(command, **_):
                instrumented = command[0] == "valgrind"
                assembly = Path(command[command.index("-o") + 1])
                assembly.write_bytes(b"bad" if instrumented and failure == "identity" else b"f: ret\n")
                if instrumented:
                    counter = Path(next(x.split("=", 1)[1] for x in command if x.startswith("--callgrind-out-file=")))
                    names = sorted(profile.cg.REQUIRED_EVENTS)
                    if failure == "events":
                        names = ["Ir"]
                    counter.write_text("events: " + " ".join(names) + "\nsummary: " + " ".join("100" for _ in names) + "\n")
                return subprocess.CompletedProcess(command, int(instrumented and failure == "exit"), b"", b"")

            with patch.object(profile.cg, "tool_identity", return_value={"mock": True}), patch.object(profile.subprocess, "run", side_effect=run), redirect_stdout(io.StringIO()):
                result = profile.main(["--mine", str(compiler), "--ref", str(compiler), "--source", str(source), "--rounds", "2", "--out", str(output)])
            return result, json.loads((output / "summary.json").read_text())

    def test_complete_equal_pair_and_alternating_order(self):
        status, report = self.measure()
        self.assertEqual(status, 0)
        self.assertTrue(report["complete"])
        self.assertEqual(report["results"][0]["median_ir_ratio"], 1)
        self.assertEqual([r["order"] for r in report["results"][0]["rounds"]], [["mine", "ref"], ["ref", "mine"]])

    def test_no_successful_subset_after_any_failed_arm(self):
        for mode in ("identity", "events", "exit"):
            with self.subTest(mode=mode):
                status, report = self.measure(mode)
                self.assertNotEqual(status, 0)
                self.assertFalse(report["complete"])
                self.assertTrue(report["error"])
