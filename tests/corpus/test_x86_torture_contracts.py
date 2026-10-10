"""Offline failure-injection contracts for the native torture measurement tool."""
from contextlib import redirect_stdout, redirect_stderr
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


torture = load("x86_gcc_torture")
evidence = load("torture_evidence")


class RunnerContract(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.suite = self.root / "gcc.c-torture/compile"
        self.suite.mkdir(parents=True)
        self.source = self.suite / "ok.c"
        self.source.write_text("int f(void) { return 0; }\n")
        self.report = self.root / "report.json"
        self.args = ["--mode=compile", "--suite", str(self.suite),
                     "--lccc", sys.executable, "--flags=-O2", "--json", str(self.report)]

    def invoke(self, extra=()):
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            return torture.main(self.args + list(extra))

    @staticmethod
    def result(case, **kwargs):
        return torture.Result(case.source.name, case.opt_flags, [], "pass", "complete", 0, 0.01)

    def test_missing_gcc_is_fatal_not_green_skips(self):
        self.assertEqual(self.invoke(["--gcc=/does/not/exist/gcc"]), 2)
        report = json.loads(self.report.read_text())
        self.assertFalse(report["complete"])
        self.assertIn("preflight", report["fatal"])
        self.assertEqual(report["completed_cases"], 0)
        self.assertIn("sha256", report["tools"]["lccc"])

    def test_all_skipped_is_not_success(self):
        result = torture.Result("ok.c", "-O2", [], "reference-compile-skip", "reference", 1, 0.)
        with mock.patch.object(torture, "reference_preflight", return_value=None), \
             mock.patch.object(torture, "run_identity", return_value={}), \
             mock.patch.object(torture, "compile_case", return_value=result):
            self.assertEqual(self.invoke(), 2)
        report = json.loads(self.report.read_text())
        self.assertTrue(report["complete"])
        self.assertIn("all-skipped", report["fatal"])

    def test_partial_checkpoint_cannot_look_complete(self):
        for i in range(25):
            (self.suite / f"case{i:02}.c").write_text("int f(void);\n")
        checkpoints = []
        with mock.patch.object(torture, "reference_preflight", return_value=None), \
             mock.patch.object(torture, "run_identity", return_value={}), \
             mock.patch.object(torture, "compile_case", side_effect=self.result), \
             mock.patch.object(torture, "atomic_json", side_effect=lambda p, data: checkpoints.append(data)):
            self.assertEqual(self.invoke(), 0)
        self.assertEqual([p["completed_cases"] for p in checkpoints], [0, 25, 26])
        self.assertEqual([p["complete"] for p in checkpoints], [False, False, True])
        self.assertTrue(all(p["expected_cases"] == 26 for p in checkpoints))

    def test_resource_failure_is_not_reference_skip(self):
        for code in (124, 125, 126, 127, -9):
            proc = subprocess.CompletedProcess([], code, b"", b"")
            self.assertEqual(torture.reference_status(proc, "compile"), "reference-fail")
        proc = subprocess.CompletedProcess([], 1, b"", b"internal compiler error")
        self.assertEqual(torture.reference_status(proc, "compile"), "reference-fail")
        proc = subprocess.CompletedProcess([], 1, b"", b"unsupported type")
        self.assertEqual(torture.reference_status(proc, "compile"), "reference-compile-skip")

    def test_multiple_flags_are_arguments_not_one_token(self):
        case = torture.Case(self.source, "-O3 -fno-inline", ())
        commands = []
        def fail(command, *args, **kwargs):
            commands.append(command)
            return subprocess.CompletedProcess(command, 1, b"", b"intentional oracle rejection")
        with mock.patch.object(torture, "run", side_effect=fail):
            torture.compile_case(case, arch="x86_64", lccc=Path(sys.executable),
                                 gcc="gcc", suite=self.suite, compile_timeout=1, append_args=[])
        self.assertIn("-O3", commands[0])
        self.assertIn("-fno-inline", commands[0])
        self.assertNotIn(case.opt_flags, commands[0])

    def test_bad_timeouts_jobs_and_duplicate_keys_rejected(self):
        for extra in (["--jobs=0"], ["--run-timeout=nan"], ["--compile-timeout=inf"],
                      ["--flags=-O2,-O2"], ["ok.c", "ok.c"], ["--flags=-O2 '"]):
            with self.subTest(extra=extra):
                self.assertEqual(self.invoke(extra), 2)

    def test_options_before_and_between_positionals(self):
        args = torture.parse_args(["--arch=i686", "one.c", "--mode=compile", "two.c"])
        self.assertEqual(args.tests, ["one.c", "two.c"])

    def test_truncated_wrong_class_and_wrong_machine_elf_rejected(self):
        obj = self.root / "test.o"
        for is_32 in (False, True):
            size = 52 if is_32 else 64
            header = bytearray(size)
            header[:7] = b"\x7fELF" + bytes((1 if is_32 else 2, 1, 1))
            header[16:20] = bytes((1, 0, 3 if is_32 else 62, 0))
            header[20] = 1
            header[40 if is_32 else 52] = size
            obj.write_bytes(header)
            self.assertTrue(torture.valid_object(obj, is_32))
            self.assertFalse(torture.valid_object(obj, not is_32))
            obj.write_bytes(header[:4])
            self.assertFalse(torture.valid_object(obj, is_32))
            header[18] = 183  # AArch64
            obj.write_bytes(header)
            self.assertFalse(torture.valid_object(obj, is_32))

    def test_hash_does_not_require_python311_file_digest(self):
        with mock.patch.object(torture.hashlib, "file_digest", create=True,
                               side_effect=AssertionError("Python 3.11-only API")):
            expected = torture.hashlib.sha256(self.source.read_bytes()).hexdigest()
            self.assertEqual(torture.file_sha256(self.source), expected)
            self.assertIsNone(torture.file_sha256(self.root / "missing"))

    def test_identity_does_not_publish_credentials(self):
        with mock.patch.dict(torture.os.environ, {
            "LCCC_FEATURE": "1", "CCC_VALIDATE_SSA": "1",
            "LCCC_API_TOKEN": "secret", "CCC_PASSWORD": "secret",
            "LCCC_API_KEY": "secret", "LCCC_AUTH": "secret",
            "OTHER": "unrelated",
        }, clear=True):
            self.assertEqual(torture.public_environment(),
                             {"CCC_VALIDATE_SSA": "1", "LCCC_FEATURE": "1"})

    def test_atomic_write_failure_preserves_previous_report(self):
        self.report.write_text("old")
        with mock.patch.object(torture.publication.os, "replace", side_effect=OSError("injected")):
            with self.assertRaises(OSError):
                torture.atomic_json(self.report, {"complete": True})
        self.assertEqual(self.report.read_text(), "old")
        self.assertFalse(list(self.root.glob("*.tmp")))

    def test_process_cwd_and_bounded_output(self):
        result = torture.run([sys.executable, "-c", "import os; print(os.getcwd())"], 2, cwd=self.root)
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.decode().strip(), str(self.root))
        result = torture.run([sys.executable, "-c", "print('x'*1000000)"], 2)
        self.assertEqual(result.returncode, 125)
        self.assertIn(b"output-limit", result.stderr)


class EvidenceContract(unittest.TestCase):
    def summarise(self, data):
        with tempfile.TemporaryDirectory() as td:
            report = Path(td) / "result.json"
            report.write_text(json.dumps(data))
            return evidence.summarise(report, "fixture")

    def test_compile_report_not_mislabelled_execute(self):
        data = {"schema": 1, "mode": "compile", "flags": ["-O2"],
                "results": [{"test": "a.c", "flags": "-O2", "status": "pass"}]}
        rows, _ = self.summarise(data)
        self.assertEqual(rows[0]["suite"], "gcc.c-torture/compile")

    def test_reference_ice_is_failed_validation_not_lccc_defect_or_skip(self):
        rows, notes = self.summarise({"schema": 1, "mode": "compile", "flags": ["-O2"],
            "results": [{"test": "a.c", "flags": "-O2", "status": "reference-fail"}]})
        self.assertEqual((rows[0]["pass_"], rows[0]["fail"], rows[0]["skipped"],
                          rows[0]["reference_fail"]), (0, 1, 0, 1))
        self.assertIn("reference", rows[0]["note"])

    def test_incomplete_or_fatal_reports_refused(self):
        data = {"schema": 2, "complete": False, "expected_cases": 5, "completed_cases": 1,
                "fatal": None, "flags": ["-O2"],
                "results": [{"test": "a.c", "flags": "-O2", "status": "pass"}]}
        with self.assertRaisesRegex(ValueError, "incomplete"):
            self.summarise(data)
        data.update(complete=True, expected_cases=1, fatal="preflight failed")
        with self.assertRaisesRegex(ValueError, "fatal"):
            self.summarise(data)
        data.update(fatal=None, expected_cases=2)
        with self.assertRaisesRegex(ValueError, "count"):
            self.summarise(data)


if __name__ == "__main__":
    unittest.main()
