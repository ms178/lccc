#!/usr/bin/env python3
"""Discharge the unresolved-`typeof` fallback follow-up with a probe matrix.

``resolve_type_spec_with_scope`` ends in ``CType::Int`` when inference fails.
The historical TODO asked for a diagnostic there, but adding one would
double-report: every probed unresolvable form is already rejected by sema with
GCC-compatible wording.  This module pins the recovery-only comment, runs a
15-shape negative matrix against GCC 16.2 / Clang 23.1.5 (and ICX when
present), and differentially checks a positive ``typeof`` matrix.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
TYPE_CHECKER = REPO / "src/frontend/sema/type_checker.rs"
LCCC = Path(os.environ.get("LCCC", REPO / "target/fastbuild/lccc"))
GCC = os.environ.get("GCC16") or shutil.which("gcc-16") or shutil.which("gcc")
CLANG = os.environ.get("CLANG") or shutil.which("clang-23")
ICX = os.environ.get("ICX") or shutil.which("icx")

CASES = {
    "undeclared": "int main(void){ typeof(undeclared) x; return 0; }",
    "undeclared_member": "int main(void){ typeof(undeclared.member) x; return 0; }",
    "undeclared_arrow": "int main(void){ typeof(undeclared->member) x; return 0; }",
    "undeclared_call": "int main(void){ typeof(undeclared_fn()) x; return 0; }",
    "undeclared_subscript": "int main(void){ typeof(undeclared[0]) x; return 0; }",
    "undeclared_addr": "int main(void){ typeof(&undeclared) x; return 0; }",
    "undeclared_deref": "int main(void){ typeof(*undeclared) x; return 0; }",
    "undeclared_nested": "int main(void){ typeof(typeof(undeclared)) x; return 0; }",
    "undeclared_sizeof": "int main(void){ typeof(sizeof undeclared) x; return 0; }",
    "bad_subscript_type": "int main(void){ int a[2]; typeof(a[undeclared]) x; return 0; }",
    "undeclared_in_cast": "int main(void){ typeof((int)undeclared) x; return 0; }",
    "undeclared_binary": "int main(void){ typeof(undeclared + 1) x; return 0; }",
    "undeclared_comma": "int main(void){ typeof((undeclared, 1)) x; return 0; }",
    "undeclared_conditional": "int main(void){ typeof(undeclared ? 1 : 2) x; return 0; }",
    "undeclared_stmtexpr": "int main(void){ typeof(({ undeclared; 1; })) x; return 0; }",
}

POSITIVE = r'''
#include <stdio.h>
struct S { int member; };
int main(void) {
    int i = 7;
    typeof(i) j = 8;
    typeof(&i) p = &i;
    typeof(*p) k = *p;
    typeof(i + j) m = i + j;
    int arr[2] = {3, 4};
    typeof(arr[0]) n = arr[1];
    struct S s = {5};
    typeof(s.member) t = s.member;
    __typeof__(t) u = t + 1;
    int r = ({ int q = 11; typeof(q) z = q + 1; z; });
    printf("%d %d %d %d %d %d %d\n", j, k, m, n, t, u, r);
    return 0;
}
'''

OLD_TODO = "// TODO: report diagnostic for unresolved typeof expression"


def compilers() -> list[str]:
    result = [str(LCCC)] if LCCC.is_file() else []
    for candidate in (GCC, CLANG, ICX):
        if candidate:
            result.append(str(candidate))
    return result


class TypeofUnresolvedFallback(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.source = TYPE_CHECKER.read_text()

    def test_todo_is_replaced_by_recovery_only_contract(self) -> None:
        self.assertNotIn(OLD_TODO, self.source)
        self.assertIn("recovery-only", self.source)
        self.assertIn("CType::Int", self.source)
        self.assertIn("double-report", self.source)

    def test_old_todo_would_fail_the_structural_ratchet(self) -> None:
        synthetic = "fn f() {\n    " + OLD_TODO + "\n    CType::Int\n}\n"
        self.assertIn(OLD_TODO, synthetic)
        self.assertNotIn("recovery-only", synthetic)

    @unittest.skipUnless(
        LCCC.is_file() and GCC and CLANG,
        "built LCCC, GCC 16.2 and Clang 23.1.5 required",
    )
    def test_every_unresolvable_shape_is_rejected_by_every_compiler(self) -> None:
        with tempfile.TemporaryDirectory(prefix="typeof-fallback-") as td:
            root = Path(td)
            for name, src in CASES.items():
                with self.subTest(case=name):
                    path = root / f"{name}.c"
                    path.write_text(src)
                    for compiler in compilers():
                        with self.subTest(compiler=Path(compiler).name):
                            proc = subprocess.run(
                                [compiler, "-std=gnu11", "-Werror", "-fsyntax-only", str(path)],
                                capture_output=True, text=True, timeout=60,
                            )
                            self.assertNotEqual(
                                proc.returncode, 0,
                                f"{compiler} accepted {name}: {proc.stdout}{proc.stderr}",
                            )
                            diagnostic = proc.stdout + proc.stderr
                            self.assertTrue(
                                "undeclared" in diagnostic
                                or "implicit declaration" in diagnostic,
                                f"{compiler} must name the unresolved operand: {diagnostic}",
                            )

    @unittest.skipUnless(LCCC.is_file(), "built LCCC required")
    def test_every_recovery_reports_itself(self) -> None:
        """The contract is observable, not merely documented.

        ``LCCC_TYPEOF_RECOVERY_TRACE`` makes each ``CType::Int`` recovery
        print the operand. The measured table below is a ratchet, not an
        aspiration: sema reports the undeclared identifier while collecting
        the declaration and then, for most shapes, abandons the declaration
        before its type is resolved, so no recovery happens at all. The
        shapes that *do* reach the resolver are exactly the ones whose
        declaration survives to type resolution.

        Two properties are load-bearing. A shape that recovers must still
        fail (a recovery is never the type of a compiling program), and the
        trace must name the operand so the recovery is actionable.
        """
        env = dict(os.environ, LCCC_TYPEOF_RECOVERY_TRACE="1")
        # Measured with this compiler; a change here is a behaviour change
        # and must be justified, not absorbed.
        expected = {
            "undeclared": 1,
            "undeclared_member": 1,
            "undeclared_arrow": 1,
            "undeclared_call": 0,
            "undeclared_subscript": 1,
            "undeclared_addr": 0,
            "undeclared_deref": 1,
            "undeclared_nested": 6,
            "undeclared_sizeof": 0,
            "bad_subscript_type": 0,
            "undeclared_in_cast": 0,
            "undeclared_binary": 0,
            "undeclared_comma": 0,
            "undeclared_conditional": 0,
            "undeclared_stmtexpr": 0,
        }
        self.assertEqual(set(expected), set(CASES), "CASES and the table disagree")
        self.assertTrue(any(expected.values()), "the trace must be exercised")
        with tempfile.TemporaryDirectory(prefix="typeof-trace-") as td:
            root = Path(td)
            for name, src in CASES.items():
                with self.subTest(case=name):
                    path = root / f"{name}.c"
                    path.write_text(src)
                    proc = subprocess.run(
                        [str(LCCC), "-std=gnu11", "-fsyntax-only", str(path)],
                        capture_output=True, text=True, timeout=60, env=env,
                    )
                    self.assertNotEqual(proc.returncode, 0, proc.stdout + proc.stderr)
                    traces = [
                        line for line in (proc.stdout + proc.stderr).splitlines()
                        if line.startswith("typeof-recovery:")
                    ]
                    self.assertEqual(
                        len(traces), expected[name],
                        f"{name}: expected {expected[name]} recoveries, got {traces}",
                    )
                    # A recovery names the operand it replaced.
                    for line in traces:
                        self.assertIn("undeclared", line)

    @unittest.skipUnless(LCCC.is_file(), "built LCCC required")
    def test_compiling_program_never_takes_the_fallback(self) -> None:
        """No recovery on any program that compiles.

        This is the property the recovery-only contract asserts, checked in
        the direction that matters: a `typeof` sema *can* type must never
        reach the fallback, or the program would be silently given `int`.
        """
        env = dict(os.environ, LCCC_TYPEOF_RECOVERY_TRACE="1")
        with tempfile.TemporaryDirectory(prefix="typeof-clean-") as td:
            root = Path(td)
            source = root / "positive.c"
            source.write_text(POSITIVE)
            proc = subprocess.run(
                [str(LCCC), "-std=gnu11", "-O2", str(source), "-o", str(root / "positive.out")],
                capture_output=True, text=True, timeout=180, env=env,
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertNotIn("typeof-recovery:", proc.stdout + proc.stderr)
            # The trace is opt-in: a normal compile stays silent.
            proc = subprocess.run(
                [str(LCCC), "-std=gnu11", "-O2", str(source), "-o", str(root / "quiet.out")],
                capture_output=True, text=True, timeout=180,
                env={k: v for k, v in os.environ.items() if k != "LCCC_TYPEOF_RECOVERY_TRACE"},
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            self.assertNotIn("typeof-recovery:", proc.stdout + proc.stderr)

    @unittest.skipUnless(
        LCCC.is_file() and GCC and CLANG,
        "built LCCC, GCC 16.2 and Clang 23.1.5 required",
    )
    def test_positive_typeof_matrix_is_bit_identical(self) -> None:
        with tempfile.TemporaryDirectory(prefix="typeof-positive-") as td:
            root = Path(td)
            source = root / "positive.c"
            source.write_text(POSITIVE)
            expected: str | None = None
            for compiler in compilers():
                with self.subTest(compiler=Path(compiler).name):
                    exe = root / f"{Path(compiler).name}.out"
                    proc = subprocess.run(
                        [compiler, "-std=gnu11", "-O2", str(source), "-o", str(exe)],
                        capture_output=True, text=True, timeout=120,
                    )
                    self.assertEqual(proc.returncode, 0, proc.stderr)
                    run = subprocess.run([str(exe)], capture_output=True, text=True, timeout=60)
                    self.assertEqual(run.returncode, 0, run.stderr)
                    if expected is None:
                        expected = run.stdout
                    self.assertEqual(run.stdout, expected)


if __name__ == "__main__":
    unittest.main()
