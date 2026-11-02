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
# The shared case identity, imported the same way both ends of the report see
# it -- a test that re-implemented the digest would prove nothing about drift.
sys.path.insert(0, str(ROOT))
from tools.corpus import evidence as shared_evidence  # noqa: E402


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
             mock.patch.object(torture, "run_identity",
                       return_value={"mode": "compile"}), \
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
             mock.patch.object(torture, "run_identity",
                       return_value={"mode": "compile"}), \
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



    def test_signal_death_is_a_reference_failure_in_every_phase(self):
        """F03: the rule cannot be limited to the compile phase.

        A reference executable killed while running produced
        `reference-run-skip`: no comparison, no failure, and the case left the
        denominator -- a hole in the evidence that read as a clean row.  An
        ordinary non-zero exit stays a skip, because that IS the corpus's own
        "this host cannot run it" signal; the distinction is death versus
        verdict, not phase one versus phase two.
        """
        for code in (-9, -11, -6, -4, 124, 125, 126, 127):
            for phase in ("compile", "run"):
                proc = subprocess.CompletedProcess([], code, b"", b"")
                with self.subTest(code=code, phase=phase):
                    self.assertEqual(torture.reference_status(proc, phase), "reference-fail")
        for phase in ("compile", "run"):
            proc = subprocess.CompletedProcess([], 1, b"", b"unsupported type")
            with self.subTest(ordinary_exit=phase):
                self.assertEqual(torture.reference_status(proc, phase),
                                 f"reference-{phase}-skip")
        proc = subprocess.CompletedProcess([], 1, b"", b"Internal Compiler Error")
        self.assertEqual(torture.reference_status(proc, "compile"), "reference-fail")
        # ICE text in the *program's* stderr is not an oracle failure; the run
        # phase reports it as an ordinary skip, exactly as before.
        self.assertEqual(torture.reference_status(proc, "run"), "reference-run-skip")

    def test_pass_plus_reference_sigkill_is_not_green(self):
        """One clean case must not carry a killed reference to a green row."""
        (self.suite / "boom.c").write_text("int f(void) { return 1; }\n")
        def outcome(case, **kwargs):
            if case.source.name == "boom.c":
                return torture.Result(case.source.name, case.opt_flags, [], "reference-fail",
                                      "reference-run", -9, 0.01, "Killed")
            return torture.Result(case.source.name, case.opt_flags, [], "pass",
                                  "complete", 0, 0.01)
        with mock.patch.object(torture, "reference_preflight", return_value=None), \
             mock.patch.object(torture, "run_identity",
                       return_value={"mode": "compile"}), \
             mock.patch.object(torture, "compile_case", side_effect=outcome):
            self.assertEqual(self.invoke(), 1)
        report = json.loads(self.report.read_text())
        self.assertEqual(report["counts"], {"pass": 1, "reference-fail": 1})
        self.assertEqual(report["completed_cases"], 2)
        self.assertTrue(report["complete"])
        self.assertIsNone(report["fatal"])
        # ... and the consumer publishes it as a failure, with the oracle
        # breakage attributed to the oracle and not to LCCC.
        rows, _ = evidence.summarise(self.report, "x86-64")
        self.assertEqual(rows[0]["fail"], 1)
        self.assertEqual(rows[0]["reference_fail"], 1)
        self.assertEqual(rows[0]["lccc_fail"], 0)
        self.assertEqual(rows[0]["total"], 2)

    def test_worker_exception_publishes_fatal_and_stops_the_queue(self):
        """F07: a runner fault is infrastructure, and must not be a traceback."""
        for index in range(4):
            (self.suite / f"c{index}.c").write_text("int f(void);\n")
        calls = []
        def flaky(case, **kwargs):
            calls.append(case.source.name)
            if len(calls) == 1:
                raise OSError("mocked ccache exploded")
            return torture.Result(case.source.name, case.opt_flags, [], "pass", "complete", 0, 0.01)
        with mock.patch.object(torture, "reference_preflight", return_value=None), \
             mock.patch.object(torture, "run_identity",
                       return_value={"mode": "compile"}), \
             mock.patch.object(torture, "compile_case", side_effect=flaky):
            self.assertEqual(self.invoke(["--jobs=1"]), 2)
        report = json.loads(self.report.read_text())
        self.assertFalse(report["complete"])
        self.assertIn("mocked ccache exploded", report["fatal"])
        self.assertIn("c0.c", report["fatal"])
        self.assertEqual(report["completed_cases"], 0)
        # The queued cases were cancelled rather than run to completion behind
        # the fault, so the toolchain is not spent on work nobody will read.
        self.assertEqual(len(calls), 1)

    def test_identity_pins_quoted_included_headers(self):
        """F06: an input the compiler reads is an input the pin must cover."""
        header = self.suite / "fixture.h"
        header.write_text("#define VALUE 0\n")
        self.source.write_text('#include "fixture.h"\nint f(void) { return VALUE; }\n')
        before = torture.dependency_digest([self.source], [self.suite])
        self.assertEqual(before["files"], 1)
        self.assertFalse(before["truncated"])
        header.write_text("#define VALUE 1\n")
        after = torture.dependency_digest([self.source], [self.suite])
        self.assertNotEqual(before["digest"], after["digest"])
        unrelated = self.suite / "other.c"
        unrelated.write_text("int unrelated(void) { return 0; }\n")
        self.assertEqual(after["digest"],
                         torture.dependency_digest([self.source], [self.suite])["digest"])

    def test_dependency_walk_reports_its_own_bound(self):
        chain = self.suite
        for index in range(4):
            (chain / f"h{index}.h").write_text(
                f'#include "h{index + 1}.h"\n' if index < 3 else "int leaf(void);\n")
        self.source.write_text('#include "h0.h"\nint f(void);\n')
        with mock.patch.object(torture, "DEPENDENCY_FILE_LIMIT", 2):
            pinned = torture.dependency_digest([self.source], [self.suite])
        self.assertTrue(pinned["truncated"])
        self.assertEqual(pinned["files"], 2)

    def test_mid_run_input_change_invalidates_the_claim(self):
        header = self.suite / "fixture.h"
        header.write_text("#define VALUE 0\n")
        self.source.write_text('#include "fixture.h"\nint f(void) { return VALUE; }\n')
        args = torture.parse_args(self.args + ["--gcc=gcc"])
        identity = {"dependency_digest": "stale", "selected_sources_sha256": "x",
                    "tools": {}, "gcc_checkout_head": None, "runner_sha256": None}
        with mock.patch.object(torture, "run", return_value=subprocess.CompletedProcess([], 0, b"", b"")):
            drift = torture.identity_drift(args, [self.source], identity, self.suite, False)
        self.assertIn("dependency_digest", drift)

    @staticmethod
    def good(results, **extra):
        keys = [f"{r['test']}[{r['flags']}]" for r in results]
        data = {"schema": 2, "complete": True, "fatal": None, "mode": "execute",
                "expected_cases": len(results), "completed_cases": len(results),
                "flags": sorted({r["flags"] for r in results}), "results": results,
                "case_set_sha256": shared_evidence.case_set_digest(keys)}
        data.update(extra)
        return data

    def test_shadowed_header_is_not_hashed_and_a_cut_identity_is_marked(self):
        """Two ways the dependency digest can be wrong, both measured here.

        (1) The search loop's condition was `is_file() and not in seen`, so an
        already-known first hit let the walk continue into the next directory and
        hash a header the compiler would never open.  (2) When the walk hit the
        cap it still returned a digest, which a report could publish as verified.
        """
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            suite = root / "suite"
            shadow = root / "shadow"
            suite.mkdir(); shadow.mkdir()
            (suite / "h.h").write_text("/* the one the compiler reads */\n")
            (shadow / "h.h").write_text("/* shadowed, must NOT be hashed */\n")
            (suite / "a.c").write_text('#include "h.h"\nint f(void);\n')
            deps, cut = torture.local_dependencies(suite / "a.c", [shadow])
            self.assertEqual([d.name for d in deps], ["h.h"])
            self.assertEqual(deps[0].parent, suite)
            self.assertFalse(cut)
            identity = torture.dependency_digest([suite / "a.c"], [shadow])
            self.assertNotIn("shadow", json.dumps(identity))
            # (2) past the cap the digest is refused, not published: files over the
            # limit are reported as a truncation, and the digest slot goes empty.
            for index in range(torture.DEPENDENCY_FILE_LIMIT + 2):
                (suite / f"i{index}.h").write_text(f"/* {index} */\n")
            (suite / "many.h").write_text("".join(f'#include "i{n}.h"\n'
                                                  for n in range(torture.DEPENDENCY_FILE_LIMIT + 2)))
            (suite / "b.c").write_text('#include "many.h"\n')
            deps2, cut2 = torture.local_dependencies(suite / "b.c", [shadow])
            self.assertTrue(cut2, "the walk must report that it stopped early")
            identity2 = torture.dependency_digest([suite / "b.c"], [shadow])
            self.assertTrue(identity2["truncated"])
            report = {**self.good([{"test": "b.c", "flags": "-O2", "status": "pass"}]),
                      "dependency_digest_truncated": True}
            self.assertTrue(any("truncated" in p for p in evidence.validate(report)),
                            "a truncated identity must not be published as verified")

    def test_producer_digest_satisfies_the_consumer(self):
        """The two ends must agree without either one being told what to expect."""
        for index in range(3):
            (self.suite / f"c{index}.c").write_text("int f(void);\n")
        def as_case(case, **kwargs):
            # A canned Result would not carry the case's own name, and the
            # point of this test is that the producer's keys and the
            # consumer's keys are the same keys.
            return torture.Result(case.source.name, case.opt_flags, [], "pass",
                                  "complete", 0, 0.01)
        with mock.patch.object(torture, "reference_preflight", return_value=None), \
             mock.patch.object(torture, "run_identity",
                       return_value={"mode": "compile"}), \
             mock.patch.object(torture, "compile_case", side_effect=as_case):
            self.assertEqual(self.invoke(), 0)
        report = json.loads(self.report.read_text())
        keys = (shared_evidence.case_key(r["test"], r["flags"]) for r in report["results"])
        self.assertEqual(shared_evidence.case_set_digest(keys), report["case_set_sha256"])
        self.assertEqual(evidence.validate(report), [])


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

    def test_empty_report_is_refused_not_published(self):
        """The hand-made report that must not validate clean and exit 0.

        `flags: []`, no rows, `expected_cases == completed_cases == 0` and the
        digest of the empty set satisfies every length rule there is, so the old
        consumer returned no problems; the CLI's refusal was guarded by `if rows`,
        so it published `{"rows": []}` and exited 0.  A reviewer reading that file
        would see a green campaign with nothing in it.
        """
        empty = {"schema": 2, "complete": True, "fatal": None, "mode": "execute",
                 "expected_cases": 0, "completed_cases": 0, "flags": [], "results": [],
                 "case_set_sha256": shared_evidence.case_set_digest([])}
        problems = evidence.validate(empty)
        self.assertTrue(problems, "an empty report must not validate")
        self.assertTrue(any("flags is empty" in p for p in problems), problems)
        self.assertTrue(any("expected_cases is 0" in p for p in problems), problems)
        with self.assertRaisesRegex(ValueError, "flags is empty"):
            self.summarise(empty)
        # The CLI is the thing CI trusts with an exit status: an empty table must
        # not be published, and must not be green.
        with tempfile.TemporaryDirectory() as td:
            report = Path(td) / "report.json"
            report.write_text(json.dumps(empty))
            out = Path(td) / "evidence.json"
            with self.assertRaises(SystemExit) as ctx:
                with redirect_stdout(io.StringIO()):
                    evidence.main([f"{report}:target", "--out", str(out)])
            self.assertEqual(ctx.exception.code, 2)
            self.assertFalse(out.exists(), "an empty evidence table must not be published")

    def test_digest_checked_even_when_nothing_was_seen(self):
        """Rows stripped from a report must not escape the very check that would show it.

        The comparison was `elif seen and digest(...) != expected`, so an empty row
        set skipped it.  The digest of the empty set is well defined, so the check
        now runs either way, and a report whose rows were removed while its
        published digest still described the full selection is caught.
        """
        rows = [{"test": f"c{n}.c", "flags": "-O2", "status": "pass"} for n in range(3)]
        full = self.good(rows)
        self.assertEqual(evidence.validate(full), [])          # control: a real report
        stripped = dict(full, results=[], expected_cases=0, completed_cases=0,
                        flags=["-O2"])
        problems = evidence.validate(stripped)
        self.assertTrue(any("case set does not match" in p for p in problems), problems)

    def test_every_declared_configuration_must_be_represented(self):
        """The mixed report: one configuration silent while the other passes.

        Totals cannot see this -- `expected == completed == len(results)` holds and
        the digest can be rebuilt for the reduced selection -- so coverage of each
        declared configuration is asserted on its own.
        """
        mixed = self.good([{"test": "a.c", "flags": "-O2", "status": "pass"}],
                          flags=["-O2", "-O3"])
        problems = evidence.validate(mixed)
        self.assertTrue(any("-O3" in p and "no rows" in p for p in problems), problems)
        both = self.good([{"test": "a.c", "flags": "-O2", "status": "pass"},
                          {"test": "a.c", "flags": "-O3", "status": "pass"}])
        self.assertEqual(evidence.validate(both), [])

    def test_truncated_dependency_identity_is_not_verified_evidence(self):
        """A digest over part of the input set is not a pin."""
        rows = [{"test": "a.c", "flags": "-O2", "status": "pass"}]
        whole = self.good(rows, dependency_digest_truncated=False)
        self.assertEqual(evidence.validate(whole), [])
        cut = self.good(rows, dependency_digest_truncated=True)
        problems = evidence.validate(cut)
        self.assertTrue(any("truncated" in p for p in problems), problems)

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


    @staticmethod
    def good(results, *, digest_over=None, **extra):
        keys = [f"{r['test']}[{r['flags']}]" for r in (digest_over or results)]
        data = {"schema": 2, "complete": True, "fatal": None, "mode": "execute",
                "expected_cases": len(results), "completed_cases": len(results),
                "flags": sorted({r["flags"] for r in results}),
                "results": results,
                "case_set_sha256": shared_evidence.case_set_digest(keys)}
        data.update(extra)
        return data

    def test_future_or_unknown_schema_cannot_be_unlocked(self):
        """F04 witness 1: length checks must not be gated on a schema we know."""
        data = self.good([{"test": "a.c", "flags": "-O2", "status": "pass"}])
        data["schema"] = 3
        data["complete"] = False
        data["expected_cases"] = 9
        with self.assertRaisesRegex(ValueError, "unsupported or missing schema"):            self.summarise(data)
        # The legacy switch is for the *older* format; it must not become the
        # way a schema this consumer has never seen gets summarised by guessing.
        # The legacy switch opens the *older* format; it must never become the
        # way a schema this consumer has not seen gets summarised by guessing.
        out = Path(tempfile.mkdtemp()) / "e.json"
        self.assertEqual(self.cli(self.write(data) + ":fixture", "--allow-legacy-schema1",
                                  "--out", str(out)), 2)

    def write(self, data):
        """A report file that lives until the test ends (the CLI re-reads it)."""
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        path = Path(temp.name) / "r.json"
        path.write_text(json.dumps(data))
        return str(path)

    def cli(self, *argv):
        """Run the consumer CLI and return (exit code, published path).

        `main` reports refusal with `SystemExit`, the way a command line tool
        must; a test that let it escape would be asserting nothing about the
        status the harness actually sees.
        """
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            try:
                code = evidence.main(list(argv))
            except SystemExit as exc:  # argparse `ap.exit`, or __main__ style
                code = exc.code
        return code

    def test_duplicate_case_cannot_fill_a_missing_one(self):
        """F04 witness 2: two identical rows are not two cases."""
        results = [{"test": "a.c", "flags": "-O2", "status": "pass"},
                   {"test": "a.c", "flags": "-O2", "status": "pass"}]
        with self.assertRaisesRegex(ValueError, "duplicate case"):
            self.summarise(self.good(results, digest_over=[{"test": "a.c", "flags": "-O2", "status": "pass"},
                                                           {"test": "b.c", "flags": "-O2", "status": "pass"}]))

    def test_undeclared_flag_cannot_hide_a_failure(self):
        """F04 witness 3: rows are built per declared flag, so verify membership."""
        results = [{"test": "a.c", "flags": "-O2", "status": "pass"},
                   {"test": "b.c", "flags": "-O3", "status": "run-fail"}]
        with self.assertRaisesRegex(ValueError, "undeclared flags"):
            self.summarise(self.good(results, flags=["-O2"]))

    def test_all_skipped_report_is_not_zero_failures(self):
        """F04 witness 4: 0 failures over 0 eligible cases is not a pass rate."""
        results = [{"test": "a.c", "flags": "-O2", "status": "reference-run-skip"}]
        rows, notes = self.summarise(self.good(results))
        self.assertEqual((rows[0]["pass_"], rows[0]["fail"], rows[0]["total"]), (0, 0, 0))
        self.assertIn("NO eligible cases", " ".join(notes))
        with tempfile.TemporaryDirectory() as td:
            report = Path(td) / "r.json"
            report.write_text(json.dumps(self.good(results)))
            out = Path(td) / "e.json"
            self.assertEqual(self.cli(f"{report}:x86-64", "--out", str(out)), 2,
                             "an evidence table of pure skips is not published")
            self.assertFalse(out.exists(), "and nothing is written on the way out")

    def test_boolean_case_counts_are_rejected(self):
        """F04 witness 5: True == 1 in Python, and is not a case count."""
        results = [{"test": "a.c", "flags": "-O2", "status": "pass"}]
        data = self.good(results * 1, expected_cases=True, completed_cases=True)
        with self.assertRaisesRegex(ValueError, "non-negative integer"):
            self.summarise(data)

    def test_row_contract_cannot_drift_from_the_producer(self):
        """The consumer names the producer's fields; a test keeps that honest."""
        import dataclasses
        required, optional = (["test", "flags", "status"],
                              ["directive_flags", "phase", "returncode", "seconds", "detail"])
        fields = sorted(f.name for f in dataclasses.fields(torture.Result))
        self.assertEqual(fields, sorted(required + optional))

    def test_valid_failing_report_is_still_publishable(self):
        """Failure data is evidence; only the *absence* of cases is refused."""
        results = [{"test": "a.c", "flags": "-O2", "status": "run-fail",
                    "directive_flags": [], "phase": "run", "returncode": 1,
                    "seconds": 0.5, "detail": "stdout mismatch"}]
        rows, notes = self.summarise(self.good(results))
        self.assertEqual((rows[0]["pass_"], rows[0]["fail"], rows[0]["lccc_fail"]), (0, 1, 1))
        with tempfile.TemporaryDirectory() as td:
            report = Path(td) / "r.json"
            report.write_text(json.dumps(self.good(results)))
            out = Path(td) / "e.json"
            self.assertEqual(self.cli(f"{report}:x86-64", "--out", str(out)), 1,
                             "a failed campaign is published, and red")
            published = json.loads(out.read_text())
        self.assertEqual(published["rows"][0]["fail"], 1)
        self.assertTrue(published["rows"][0]["verified"])

    def test_legacy_schema_needs_the_explicit_flag_and_is_labelled(self):
        data = {"schema": 1, "mode": "compile", "flags": ["-O2"],
                "results": [{"test": "a.c", "flags": "-O2", "status": "pass"}]}
        rows, notes = self.summarise(data)
        self.assertFalse(rows[0]["verified"])
        self.assertIn("archival", " ".join(notes))
        with tempfile.TemporaryDirectory() as td:
            report = Path(td) / "r.json"
            report.write_text(json.dumps(data))
            out = Path(td) / "e.json"
            self.assertEqual(self.cli(f"{report}:legacy", "--out", str(out)), 2,
                             "the unchecked archival format is refused by default")
            self.assertFalse(out.exists())
            self.assertEqual(self.cli(f"{report}:legacy", "--out", str(out),
                                      "--allow-legacy-schema1"), 0)
            published = json.loads(out.read_text())
            self.assertFalse(published["rows"][0]["verified"])


if __name__ == "__main__":
    unittest.main()
