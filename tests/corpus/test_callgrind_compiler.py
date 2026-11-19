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

    def measure(self, mine, ref, *, extra=(), **overrides):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith("LCCC_CG_") and k != "TEST_UNDER_CALLGRIND"}
        env["PATH"] = os.pathsep.join((str(self.root), env["PATH"]))
        env.update(overrides)
        run = subprocess.run(
            [sys.executable, str(SCRIPT), "--mine", str(mine), "--ref", str(ref),
             "--source", str(self.source), "--rounds", "2", "--out", str(self.out), *extra],
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

    def test_bad_run_options_invalidate_a_previous_complete_report(self):
        mine, ref = self.compiler("mine"), self.compiler("ref")
        for extra, diagnostic in (
            (("--rounds", "0"), "rounds and timeout must be positive"),
            (("--timeout", "0"), "rounds and timeout must be positive"),
            (("--flags=-oother.s",), "output/mode flags are owned by the harness"),
            (("--flags='-O1",), "No closing quotation"),
        ):
            with self.subTest(extra=extra):
                run, report = self.measure(mine, ref)
                self.assertEqual(run.returncode, 0, run.stderr)
                self.assertTrue(report["complete"])
                run, report = self.measure(mine, ref, extra=extra)
                self.assertFalse(report["complete"], "failed setup left a stale success")
                self.assertEqual(run.returncode, 1, run.stderr)
                self.assertEqual(report["results"], [])
                self.assertIn(diagnostic, report["error"])
                self.assertIn(diagnostic, run.stderr)

    def test_an_input_in_a_frozen_output_slot_cannot_be_replaced_by_the_other_arm(self):
        mine, ref = self.compiler("mine"), self.compiler("ref", "nop\nret\n")
        owned = self.out / "aa/compiler"
        owned.parent.mkdir(parents=True)
        owned.write_bytes(ref.read_bytes())
        owned.chmod(0o755)
        run, report = self.measure(mine, owned)
        self.assertEqual(owned.read_bytes(), ref.read_bytes(), "freezing mine rewrote ref")
        self.assertEqual(run.returncode, 1, run.stderr)
        self.assertFalse(report["complete"])
        self.assertIn("reserved compiler-work output directory", report["error"])
        self.assertEqual(report["results"], [])

    def test_a_hard_link_to_a_stale_frozen_compiler_does_not_change_the_reference(self):
        mine, ref = self.compiler("mine"), self.compiler("ref", "nop\nret\n")
        owned = self.out / "aa/compiler"
        owned.parent.mkdir(parents=True)
        owned.hardlink_to(ref)
        original = ref.read_bytes()
        run, report = self.measure(mine, ref)
        self.assertEqual(ref.read_bytes(), original, "freezing mine changed the reference's inode")
        self.assert_rejected(run, report, "native assembly differs")

    def test_an_arm_directory_symlink_does_not_redirect_profile_writes(self):
        mine, ref = self.compiler("mine"), self.compiler("ref")
        original = self.root / "unrelated"
        original.mkdir()
        self.out.mkdir()
        (self.out / "aa").symlink_to(original, target_is_directory=True)
        run, report = self.measure(mine, ref)
        self.assertEqual(run.returncode, 1, run.stderr)
        self.assertFalse(report["complete"])
        self.assertIn("output directory must not be a symlink", report["error"])
        self.assertEqual(list(original.iterdir()), [])

    def test_writing_a_new_stderr_receipt_does_not_change_an_input_inode(self):
        mine, ref = self.compiler("mine"), self.compiler("ref")
        stderr_path = self.out / "aa/case-0-round-0.stderr"
        stderr_path.parent.mkdir(parents=True)
        stderr_path.hardlink_to(self.source)
        original = self.source.read_bytes()
        run, report = self.measure(mine, ref)
        self.assertEqual(run.returncode, 0, run.stderr)
        self.assertTrue(report["complete"])
        self.assertEqual(self.source.read_bytes(), original)
        self.assertEqual(stderr_path.read_bytes(), b"")

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
