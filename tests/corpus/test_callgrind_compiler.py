"""Exercise the compiler-work identity gate without a compiler or Valgrind."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[2] / "scripts/callgrind_compiler.py"
VALGRIND = """
import os
from pathlib import Path
import subprocess
import sys

args = sys.argv[1:]
if args == ['--version']:
    print('valgrind-test-double')
    sys.exit(0)
for index, arg in enumerate(args):
    if arg.startswith('--callgrind-out-file='):
        output = Path(arg.split('=', 1)[1])
        break
else:
    sys.exit('missing Callgrind output path')
rc = subprocess.call(args[index + 1:], env=dict(os.environ, TEST_UNDER_CALLGRIND='1'))
if rc == 0:
    output.write_text('events: Ir I1mr D1mr D1mw ILmr DLmr DLmw Bcm Bim\\n'
                      'summary: 100 1 0 0 1 0 0 0 0\\n')
sys.exit(rc)
"""


class CompilerWorkIdentity(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "input.c"
        self.source.write_text("void f(void) {}\n")
        self.out = self.root / "report"
        self.executable("valgrind", VALGRIND)

    def executable(self, name, body):
        path = self.root / name
        path.write_text(f"#!{sys.executable}\n{body}")
        path.chmod(0o755)
        return path

    def compiler(self, name, asm="ret\n", stdout="", *, instrumented_asm=None, round_stdout=None):
        return self.executable(name, f"""
import os
from pathlib import Path
import sys

instrumented = {instrumented_asm!r}
asm = instrumented if instrumented is not None and os.getenv('TEST_UNDER_CALLGRIND') else {asm!r}
Path(sys.argv[sys.argv.index('-o') + 1]).write_text(asm)
outputs = {round_stdout!r}
stdout = {stdout!r}
if outputs is not None:
    counter = Path(__file__).with_name('invocations')
    count = int(counter.read_text()) if counter.exists() else 0
    counter.write_text(str(count + 1))
    stdout = outputs[count // 2]
sys.stdout.write(stdout)
""")

    def measure(self, mine, ref, **overrides):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith("LCCC_CG_") and k != "TEST_UNDER_CALLGRIND"}
        env["PATH"] = os.pathsep.join((str(self.root), env["PATH"]))
        env.update(overrides)
        run = subprocess.run(
            [sys.executable, str(SCRIPT), "--mine", str(mine), "--ref", str(ref),
             "--source", str(self.source), "--rounds", "2", "--out", str(self.out)],
            env=env, cwd=self.root, capture_output=True, text=True, timeout=20,
        )
        self.assertTrue((self.out / "summary.json").is_file(), run.stderr)
        return run, json.loads((self.out / "summary.json").read_text())

    def assert_rejected(self, run, report, error):
        self.assertEqual(run.returncode, 1, run.stderr)
        self.assertFalse(report["complete"])
        self.assertIn(error, report["error"])
        self.assertNotIn("ir_ratio", report["results"][0]["rounds"][0])

    def test_identical_arms_publish_both_orders(self):
        stdout = "observed\n"
        run, report = self.measure(self.compiler("mine", stdout=stdout), self.compiler("ref", stdout=stdout))
        self.assertEqual(run.returncode, 0, report["error"])
        self.assertTrue(report["complete"])
        rounds = report["results"][0]["rounds"]
        self.assertEqual([r["order"] for r in rounds], [["mine", "ref"], ["ref", "mine"]])
        self.assertEqual([r["ir_ratio"] for r in rounds], [1.0, 1.0])
        self.assertEqual({arm["stdout_sha256"] for r in rounds for arm in r["arms"].values()},
                         {hashlib.sha256(stdout.encode()).hexdigest()})

    def test_individually_stable_arms_with_different_assembly_are_rejected(self):
        run, report = self.measure(self.compiler("mine", "nop\nret\n"), self.compiler("ref"))
        self.assert_rejected(run, report, "native assembly differs")
        self.assertIn("input.c", report["error"])

    def test_individually_stable_arms_with_different_stdout_are_rejected(self):
        run, report = self.measure(self.compiler("mine", stdout="different\n"), self.compiler("ref"))
        self.assert_rejected(run, report, "native stdout differs")

    def test_instrumentation_only_divergence_is_rejected(self):
        run, report = self.measure(self.compiler("mine", instrumented_asm="nop\nret\n"),
                                   self.compiler("ref"))
        self.assert_rejected(run, report, "native/instrumented compiler output mismatch")

    def test_drift_shared_by_both_arms_is_rejected_in_the_second_round(self):
        outputs = ("old\n", "new\n")
        run, report = self.measure(self.compiler("mine", round_stdout=outputs),
                                   self.compiler("ref", round_stdout=outputs))
        self.assertEqual(run.returncode, 1, run.stderr)
        self.assertFalse(report["complete"])
        self.assertIn("round 2, ref: native stdout differs", report["error"])
        row, = report["results"]
        self.assertEqual(row["rounds"][0]["ir_ratio"], 1.0)
        self.assertNotIn("ir_ratio", row["rounds"][1])
        self.assertNotIn("median_ir_ratio", row)

    def test_failed_preflight_invalidates_a_previous_complete_report(self):
        mine, ref = self.compiler("mine"), self.compiler("ref")
        for error in ("missing source", "invalid geometry", "missing Valgrind"):
            with self.subTest(error=error):
                run, report = self.measure(mine, ref)
                self.assertEqual(run.returncode, 0, run.stderr)
                self.assertTrue(report["complete"])
                if error == "missing source":
                    self.source.unlink()
                    run, report = self.measure(mine, ref)
                    self.source.write_text("void f(void) {}\n")
                    diagnostic = str(self.source)
                elif error == "invalid geometry":
                    run, report = self.measure(mine, ref, LCCC_CG_I1="invalid")
                    diagnostic = "invalid I1 cache geometry"
                else:
                    run, report = self.measure(mine, ref, PATH=str(self.root / "empty"))
                    diagnostic = "tool unavailable: valgrind"
                self.assertEqual(run.returncode, 1, run.stderr)
                self.assertFalse(report["complete"])
                self.assertEqual(report["results"], [])
                self.assertIn(diagnostic, report["error"])
