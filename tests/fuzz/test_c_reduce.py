#!/usr/bin/env python3
"""Mutation tests for scripts/c_reduce.py and scripts/c_reduce_predicate.py (T4).

Run: python3 -m unittest discover -s tests/fuzz -p 'test_c_reduce.py'

Each test is a mutation check: it fails if the reducer or predicate is broken
in the way it is meant to catch.  The end-to-end cases use a fake compiler
with a known miscompile (tests/fuzz/fake_miscompiler.py), so the expected
witness is known in advance.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import c_reduce  # noqa: E402

FAKE = HERE / "fake_miscompiler.py"
PRED = ROOT / "scripts" / "c_reduce_predicate.py"
CLI = ROOT / "scripts" / "c_reduce.py"

PROGRAM = """\
#include <stdio.h>
static int helper_a(int v) { return v + 11; }
static int helper_b(int v) { return v ^ 0x55; }
static int unused_table[8] = {1, 2, 3, 4, 5, 6, 7, 8};
int main(void) {
    int acc = 0;
    for (int i = 0; i < 4; i++) acc += helper_a(i);
    acc = helper_b(acc);
    int x = 7;
    printf("noise %d\\n", acc + unused_table[2]);
    printf("value %d\\n", x * 3);
    return 0;
}
"""


def sanitizer_capable(cc: str) -> bool:
    """True when `cc` builds and runs the predicate's sanitized probe.

    The pinned oracle compilers are compiler-only snapshots: they carry the
    sanitizer headers but not `libasan`/`libubsan`, so `gcc` resolving to GCC
    16 (the CI preamble shadows it) cannot link the reference oracle at all.
    Picking by name alone therefore selected a reference that rejected EVERY
    candidate -- and the reducer read that as "not interesting" and reduced
    nothing.  Select by capability, and let the predicate's own probe (which
    also guards the standalone CLI) be the single definition of "capable".
    """
    import c_reduce_predicate

    return c_reduce_predicate.probe_reference(cc, 20.0) is None


def reference_compiler() -> str | None:
    """First reference compiler that can actually run the sanitized oracle."""
    candidates = [os.environ["REF_CC"]] if os.environ.get("REF_CC") else []
    candidates += ["gcc", "cc", "clang"]
    seen: set[str] = set()
    for candidate in candidates:
        path = shutil.which(candidate)
        if path is None or path in seen:
            continue
        seen.add(path)
        if sanitizer_capable(path):
            return path
    return None


REF = reference_compiler()


class DdminUnit(unittest.TestCase):
    def test_result_is_one_minimal_for_two_needed_lines(self):
        """ddmin keeps exactly the two lines the predicate needs, no more."""
        lines = [f"line {i}\n" for i in range(40)]
        lines[7] = "NEEDED-A\n"
        lines[31] = "NEEDED-B\n"
        with tempfile.TemporaryDirectory() as tmp:
            def check(path: Path) -> bool:
                text = path.read_text()
                return "NEEDED-A" in text and "NEEDED-B" in text
            red = c_reduce.Reducer(check, Path(tmp), 10_000, lines)
            out = red.one_minimal(red.ddmin(lines))
        self.assertEqual(sorted(out), ["NEEDED-A\n", "NEEDED-B\n"])

    def test_budget_returns_accepted_progress_not_input(self):
        """A budget stop must keep reductions already accepted (mutation: returning the input)."""
        lines = [f"x{i}\n" for i in range(64)]
        lines[0] = "KEEP\n"
        with tempfile.TemporaryDirectory() as tmp:
            def check(path: Path) -> bool:
                return "KEEP" in path.read_text()
            out, calls, exhausted = c_reduce.reduce_text(
                "".join(lines), check, max_tests=3, scratch=Path(tmp))
        self.assertTrue(exhausted)
        self.assertLessEqual(calls, 3)
        self.assertIn("KEEP", out)
        self.assertLess(len(out.splitlines()), len(lines))

    def test_memoisation_skips_repeat_candidates(self):
        """Identical candidates are answered from the cache, not re-run."""
        lines = ["a\n", "b\n", "c\n", "d\n"]
        with tempfile.TemporaryDirectory() as tmp:
            red = c_reduce.Reducer(lambda p: "a" in p.read_text(), Path(tmp), 100, lines)
            red.interesting(["a\n"])
            red.interesting(["a\n"])
            self.assertEqual(red.tests, 1)


@unittest.skipUnless(
    REF is not None,
    "needs a reference C compiler with a working -fsanitize=undefined,address runtime",
)
class EndToEnd(unittest.TestCase):
    def _run(self, env_extra, body=PROGRAM):
        tmp = tempfile.mkdtemp(prefix="c-reduce-e2e-")
        self.addCleanup(shutil.rmtree, tmp, True)
        src = Path(tmp) / "case.c"
        src.write_text(body)
        out = Path(tmp) / "min.c"
        env = dict(os.environ, **env_extra)
        proc = subprocess.run(
            [sys.executable, str(CLI), str(src), "--out", str(out), "--",
             sys.executable, str(PRED), "--ccc", str(FAKE), "--ref", str(REF),
             "--level=-O2"],
            env=env, capture_output=True, text=True, timeout=600, check=False)
        return proc, out

    def test_reduces_known_miscompile_to_a_small_witness(self):
        proc, out = self._run({"FAKE_MISCOMPILE": "1"})
        self.assertEqual(proc.returncode, 0, proc.stderr)
        reduced = out.read_text()
        self.assertIn("* 3", reduced)
        self.assertLessEqual(len(reduced.splitlines()), 8, reduced)
        self.assertLess(len(reduced), len(PROGRAM) // 2)

    def test_correct_compiler_is_not_interesting(self):
        """Negative control: a correct compiler gives exit 2, no reduction."""
        proc, out = self._run({"FAKE_MISCOMPILE": "0"})
        self.assertEqual(proc.returncode, 2, proc.stderr)
        self.assertFalse(out.exists())

    def test_ub_candidate_is_rejected_by_reference_oracle(self):
        """A program with out-of-bounds access must not be blamed on the compiler.

        The fake miscompiles, but the reference (ASan) rejects the input first.
        """
        ub = PROGRAM.replace("unused_table[2]", "unused_table[99]")
        proc, _ = self._run({"FAKE_MISCOMPILE": "1"}, body=ub)
        self.assertEqual(proc.returncode, 2, proc.stderr)


class ScratchPath(unittest.TestCase):
    def test_reducer_uses_one_stable_scratch_path(self):
        """The predicate runs synchronously, so every candidate must be
        presented through the one scratch file (no path alternation left
        behind from a hypothetical concurrent mode)."""
        seen: set[str] = set()
        lines = [f"x{i}\n" for i in range(8)]
        with tempfile.TemporaryDirectory() as tmp:
            def check(path: Path) -> bool:
                seen.add(path.name)
                return "x0" in path.read_text()
            red = c_reduce.Reducer(check, Path(tmp), 100, lines)
            red.one_minimal(red.ddmin(list(lines)))
        self.assertEqual(seen, {"cand.c"})


@unittest.skipUnless(
    REF is not None,
    "needs a reference C compiler with a working -fsanitize=undefined,address runtime",
)
class LeakTolerance(unittest.TestCase):
    """The reference oracle must accept a defined-but-leaking candidate.

    -fsanitize=address enables LeakSanitizer, which forces exit status 1 for
    ANY allocation still alive at exit.  Leaks are not undefined behaviour,
    so without ASAN_OPTIONS=detect_leaks=0 the oracle would reject exactly
    the candidates a reducer most often produces (an initialiser-with-free
    line deleted, a leak left behind).
    """

    # The ``7 * 3`` is the fake miscompiler's known target: it becomes
    # ``7 * 4`` under FAKE_MISCOMPILE=1, so an honest judge reports
    # interesting (stdout differs) only if the reference run accepted the
    # leaking-but-defined program in the first place.
    LEAK = (
        "#include <stdlib.h>\n#include <stdio.h>\n"
        "int main(void) { void *p = malloc(4096); (void)p;\n"
        "    printf(\"kept %d\\n\", 7 * 3); return 0; }\n"
    )

    _asan_probe: bool | None = None

    @classmethod
    def _have_working_asan(cls) -> bool:
        if cls._asan_probe is None:
            with tempfile.TemporaryDirectory() as tmp:
                src = Path(tmp) / "p.c"
                exe = Path(tmp) / "p.exe"
                src.write_text(cls.LEAK)
                built = subprocess.run(
                    [str(REF), "-fsanitize=undefined,address", "-fno-sanitize-recover=all",
                     str(src), "-o", str(exe), "-lm"],
                    capture_output=True, timeout=120, check=False)
                cls._asan_probe = built.returncode == 0
        return cls._asan_probe

    def test_leaking_candidate_stays_interesting(self):
        if not self._have_working_asan():
            self.skipTest("reference compiler cannot build ASan programs")
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "leak.c"
            src.write_text(self.LEAK)
            env = dict(os.environ, FAKE_MISCOMPILE="1")
            proc = subprocess.run(
                [sys.executable, str(PRED), "--ccc", str(FAKE), "--ref", str(REF),
                 "--kind", "miscompile", str(src)],
                env=env, capture_output=True, text=True, timeout=300, check=False)
            self.assertEqual(proc.returncode, 0,
                             f"defined-but-leaking candidate rejected: {proc.stderr}")

    def test_leak_sanitizer_itself_is_still_armed(self):
        """Guard the guard: if this host's ASan stops reporting leaks by
        default, the fix above would silently become a no-op assumption."""
        if not self._have_working_asan():
            self.skipTest("reference compiler cannot build ASan programs")
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "leak.c"
            exe = Path(tmp) / "leak.exe"
            src.write_text(self.LEAK)
            subprocess.run(
                [str(REF), "-fsanitize=undefined,address", "-fno-sanitize-recover=all",
                 str(src), "-o", str(exe), "-lm"],
                capture_output=True, timeout=120, check=True)
            default = subprocess.run([str(exe)], capture_output=True, timeout=60,
                                     check=False)
            self.assertNotEqual(default.returncode, 0,
                                "LSan did not fire by default; the detect_leaks=0 "
                                "rationale needs re-checking on this host")
            quiet = subprocess.run([str(exe)], capture_output=True, timeout=60,
                                   check=False,
                                   env={**os.environ, "ASAN_OPTIONS": "detect_leaks=0"})
            self.assertEqual(quiet.returncode, 0,
                             "detect_leaks=0 did not disarm LSan on this host")


class PredicateContract(unittest.TestCase):
    def test_predicate_usage_error_aborts_instead_of_meaning_not_interesting(self):
        """Mutation: a predicate exiting 2 must abort the reduction (exit 1), not be read as a verdict."""
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "c.c"
            src.write_text("int main(void){return 0;}\n")
            out = Path(tmp) / "o.c"
            proc = subprocess.run(
                [sys.executable, str(CLI), str(src), "--out", str(out), "--",
                 sys.executable, "-c", "import sys; sys.exit(2)"],
                capture_output=True, text=True, timeout=120, check=False)
            self.assertEqual(proc.returncode, 1, proc.stderr)
            self.assertIn("expected 0 or 1", proc.stderr)
            self.assertFalse(out.exists())


    def test_missing_compiler_under_test_is_an_error_not_a_crash(self):
        """Mutation: a missing `--ccc` must not count as a crash (that reduces to nothing)."""
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "c.c"
            src.write_text("int main(void){return 0;}\n")
            proc = subprocess.run(
                [sys.executable, str(PRED), "--ccc", str(Path(tmp) / "no-such-cc"),
                 "--ref", str(REF), "--kind", "crash", str(src)],
                capture_output=True, text=True, timeout=120, check=False)
            self.assertEqual(proc.returncode, 2, proc.stderr)
            self.assertIn("cannot execute", proc.stderr)


class ReferenceProbeContract(unittest.TestCase):
    """Mutation: a reference without sanitizer support must be a SETUP error.

    Before the probe existed, such a compiler made the predicate answer
    "not interesting" (exit 1) for every candidate -- and a missing `--ccc`
    was never reached, so the reducer reported a clean "nothing to minimise".
    """

    def test_unusable_reference_is_exit_2_not_a_verdict(self) -> None:
        import c_reduce_predicate

        tmp = tempfile.mkdtemp(prefix="c-reduce-probe-")
        self.addCleanup(shutil.rmtree, tmp, True)
        root = Path(tmp)
        fake = root / "no-sanitizers"
        fake.write_text("#!/bin/sh\nexit 1\n")
        fake.chmod(0o755)
        src = root / "c.c"
        src.write_text("int main(void){return 0;}\n")
        proc = subprocess.run(
            [sys.executable, str(PRED), "--ccc", str(FAKE), "--ref", str(fake),
             str(src)],
            capture_output=True, text=True, timeout=120, check=False)
        self.assertEqual(proc.returncode, 2, proc.stderr)
        self.assertIn("reference compiler", proc.stderr)
        self.assertIsNotNone(c_reduce_predicate.probe_reference(str(fake), 20.0))

    def test_pinned_oracle_is_detected_as_unusable_when_it_lacks_runtimes(self) -> None:
        """The compiler-only snapshots must be REJECTED, not silently accepted."""
        import c_reduce_predicate

        tmp = tempfile.mkdtemp(prefix="c-reduce-probe-")
        self.addCleanup(shutil.rmtree, tmp, True)
        fake = Path(tmp) / "compiler-only"
        fake.write_text(
            "#!/bin/sh\n"
            "for arg in \"$@\"; do\n"
            "  case \"$arg\" in -fsanitize=*) echo 'cannot find -lasan' >&2; exit 1;; esac\n"
            "done\n"
            "exit 1\n"
        )
        fake.chmod(0o755)
        self.assertIsNotNone(c_reduce_predicate.probe_reference(str(fake), 20.0))


if __name__ == "__main__":
    unittest.main()
