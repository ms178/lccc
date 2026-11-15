#!/usr/bin/env python3
"""Pin the multi-range byte-classifier codegen and its measured ceiling.

`k_classify.c` and `k_namechars.c` were the widest runtime gap against
GCC 16.2 (0.19x / 0.25x at `-O3`) until the chain-aware flattening landed
(PR #1185; see the 2026-11-13 update in
`engineering/BENCH-2026-11-11-BN-MUL128-CLOSURE.md` — now 1.17x / 3.59x).
The original cause was measured, not guessed: the `a||b||c||d||e||f`
predicate lowered to a partially if-converted loop -- `orl` of the two
letter-range `setcc` results, then a data-dependent `je`, with the
digit/equality half as a `Select` chain -- and the loop vectorizer bailed on
the residual branch, while the same predicate written branchlessly ran 5.2x
faster.

This module keeps that state of the art checkable so the winning path cannot
rot silently:

1. **Correctness, both forms, every compiler.** The branchy and the branchless
   predicate must agree with GCC/Clang on every byte 0..255, at `-O0` and
   `-O3`. A regression here is a miscompile, whatever the speed.
2. **The vectorizer path stays open.** The branchless form must still be
   vectorized by LCCC (packed byte compares, no scalar setcc chain). If a
   future change closes that path, the ceiling this document measures becomes
   unreachable, and the test says so instead of leaving it to a benchmark run.
3. **The branchy form stays vectorized.** The original ratchet asserted "not
   vectorized" and instructed its own flip when the chain flattening landed;
   that flip happened with the 2026-11-13 measurement, and the assertion now
   guards the landed path against regression.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
BENCH = REPO / "tests" / "bench"
LCCC = Path(os.environ.get("LCCC", REPO / "target/fastbuild/lccc"))
GCC = os.environ.get("GCC16") or shutil.which("gcc-16") or shutil.which("gcc")
CLANG = os.environ.get("CLANG") or shutil.which("clang-23")
ICX = os.environ.get("ICX") or shutil.which("icx")

# One byte value per arm; the kernel's own alphabet covers the letters, digits
# and separators, and the harness walks the rest.
BYTE_PROBE = r'''
#include <stdio.h>
static int is_name_char_branchy(char ch) {
    return ((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
            (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.');
}
static int is_name_char_branchless(char ch) {
    return ((ch >= 'a' && ch <= 'z') | (ch >= 'A' && ch <= 'Z') |
            (ch >= '0' && ch <= '9') | (ch == '_') | (ch == '-') | (ch == '.'));
}
int main(void) {
    unsigned long long a = 0, b = 0;
    for (int i = -128; i < 128; i++) {
        a = a * 131 + (unsigned long long)(unsigned) is_name_char_branchy((char) i);
        b = b * 131 + (unsigned long long)(unsigned) is_name_char_branchless((char) i);
    }
    printf("%llu %llu\n", a, b);
    return a == b ? 0 : 1;
}
'''

# The branchless kernel: same predicate, bitwise `|` instead of `||`.
BRANCHLESS_KERNEL = r'''
#include <stddef.h>
#define N 4096
static char s[N];
void bench_setup(void) {
    const char *alpha = "abcXYZ019_-.<>&\" \t\n";
    for (int i = 0; i < N; i++) s[i] = alpha[i % 19];
}
static int is_name_char(char ch) {
    return ((ch >= 'a' && ch <= 'z') | (ch >= 'A' && ch <= 'Z') |
            (ch >= '0' && ch <= '9') | (ch == '_') | (ch == '-') | (ch == '.'));
}
unsigned long long bench_run(void) {
    unsigned long long c = 0;
    for (int i = 0; i < N; i++) c += (unsigned) is_name_char(s[i]);
    return c;
}
'''


def tool(path: str | None) -> str | None:
    return path if path and Path(str(path)).is_file() else None


def compilers() -> list[str]:
    found = [str(LCCC)] if LCCC.is_file() else []
    for candidate in (GCC, CLANG, ICX):
        if tool(candidate):
            found.append(str(candidate))
    return found


def build_and_run(compiler: str, source: Path, opt: str, extra: list[str] | None = None) -> subprocess.CompletedProcess:
    exe = source.with_suffix(f".{Path(compiler).name}{opt.strip('-')}.out")
    compile_proc = subprocess.run(
        [compiler, f"-{opt}" if not opt.startswith("-") else opt, "-std=gnu11",
         str(source), "-o", str(exe)] + (extra or []),
        capture_output=True, text=True, timeout=180,
    )
    if compile_proc.returncode != 0:
        return compile_proc
    return subprocess.run([str(exe)], capture_output=True, text=True, timeout=120)


@unittest.skipUnless(LCCC.is_file(), "built LCCC required")
class ClassifierPredicateContract(unittest.TestCase):
    def test_both_predicate_forms_agree_on_every_byte(self) -> None:
        """Correctness first, at both ends of the optimization range."""
        with tempfile.TemporaryDirectory(prefix="classify-") as td:
            source = Path(td) / "probe.c"
            source.write_text(BYTE_PROBE)
            for opt in ("-O0", "-O3"):
                for compiler in compilers():
                    with self.subTest(compiler=Path(compiler).name, opt=opt):
                        run = build_and_run(compiler, source, opt)
                        self.assertEqual(run.returncode, 0, f"{compiler} {opt}: {run.stdout}{run.stderr}")
                        parts = run.stdout.split()
                        self.assertEqual(len(parts), 2, run.stdout)
                        self.assertEqual(
                            parts[0], parts[1],
                            f"{compiler} {opt}: branchy and branchless predicates disagree",
                        )

    def test_branchless_predicate_still_vectorizes(self) -> None:
        """The 5.2x ceiling depends on this loop staying vectorized."""
        with tempfile.TemporaryDirectory(prefix="classify-vec-") as td:
            source = Path(td) / "branchless.c"
            source.write_text(BRANCHLESS_KERNEL)
            asm = Path(td) / "branchless.s"
            proc = subprocess.run(
                [str(LCCC), "-O3", "-S", "-o", str(asm), str(source)],
                capture_output=True, text=True, timeout=180,
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            text = asm.read_text()
            packed = [
                mnemonic
                for mnemonic in ("pcmpgtb", "pcmpeqb", "pand", "por", "paddb")
                if mnemonic in text
            ]
            self.assertTrue(
                packed,
                "the branchless classifier must reach the vectorizer "
                f"(no packed byte compare in the emitted assembly); got {packed}",
            )
            # Measured on the vectorizing path: 13 packed constants are
            # materialised into xmm registers, which leaves no room for the
            # temporaries, so the loop spills 12 vector values and reloads 18
            # per iteration. SSE2 packed ALU ops accept an m128 memory
            # operand and the pool is 16-byte aligned, so those constants
            # never need a register -- `lane_const_rip_operand` already emits
            # exactly that form for the unsigned-compare bias. This is a
            # downward-only ratchet: it fails if the spilling gets worse, and
            # is lowered (to zero, which is the target) when the constants
            # move into memory operands.
            spills = len(re.findall(r"movd(?:qu|qa)\s+%xmm\d+,\s*-?\d+\(%rsp\)", text))
            reloads = len(re.findall(r"movd(?:qu|qa)\s+-?\d+\(%rsp\),", text))
            self.assertLessEqual(
                spills, 12,
                f"vector spill count grew to {spills} (was 12); the packed "
                "constants are competing with the temporaries for xmm registers",
            )
            self.assertLessEqual(
                reloads, 18,
                f"vector reload count grew to {reloads} (was 18)",
            )

    def test_branchy_classifier_is_vectorized(self) -> None:
        """The flipped ratchet: the chain flattening landed (PR #1185).

        `if_convert`'s `body_becomes_branch_free()` now simulates the later
        combine rounds and `vectorize_byte_count_normalize` flattens the
        residual pure value triangles, so the branchy `a||b||...` predicate
        reaches the same packed byte-compare reduction as the branchless
        rewrite. Re-measured per the original ratchet's instruction
        (driver 2000, best of 5, checksums equal to GCC in every arm):
        classify 1.17x GCC 16.2 at -O3 (was 0.19x), namechars 3.59x (was
        0.25x); see the 2026-11-13 update in
        engineering/BENCH-2026-11-11-BN-MUL128-CLOSURE.md. If this assertion
        fires, the flattening path regressed and the benchmark gap is back —
        do not weaken it without a new measurement in that document.
        """
        source = BENCH / "k_classify.c"
        if not source.is_file():
            self.skipTest("k_classify.c not present")
        with tempfile.TemporaryDirectory(prefix="classify-packed-") as td:
            asm = Path(td) / "classify.s"
            proc = subprocess.run(
                [str(LCCC), "-O3", "-S", "-o", str(asm), str(source), "-I", str(BENCH)],
                capture_output=True, text=True, timeout=180,
            )
            self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
            text = asm.read_text()
            self.assertIn(
                "pcmpgtb", text,
                "the branchy classifier is no longer vectorized: the chain "
                "flattening (if_convert combine simulation + "
                "vectorize_byte_count_normalize triangle flatten) regressed. "
                "Re-measure k_classify/k_namechars against "
                "engineering/BENCH-2026-11-11-BN-MUL128-CLOSURE.md before "
                "touching this assertion.",
            )
            self.assertIn(
                "psadbw", text,
                "packed compares exist but the counting reduction no longer "
                "reaches the psadbw accumulator",
            )

    def test_disable_switches_do_not_change_the_result(self) -> None:
        """The kill switches must bisect performance, never correctness."""
        source = BENCH / "k_classify.c"
        if not source.is_file():
            self.skipTest("k_classify.c not present")
        with tempfile.TemporaryDirectory(prefix="classify-kill-") as td:
            driver = BENCH / "driver.c"
            self.assertTrue(driver.is_file(), "shared benchmark driver missing")
            outputs = []
            for passes in ("", "if_convert", "set_membership", "range_fold"):
                env = dict(os.environ)
                if passes:
                    env["CCC_DISABLE_PASSES"] = passes
                exe = Path(td) / f"k{passes or 'all'}.out"
                # The kernel has no `main`: the shared driver supplies it, so
                # every arm runs the same harness and prints
                # "<seconds> <checksum>".
                proc = subprocess.run(
                    [str(LCCC), "-O3", str(source), str(driver), "-o", str(exe),
                     "-I", str(BENCH)],
                    capture_output=True, text=True, timeout=180, env=env,
                )
                self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
                run = subprocess.run([str(exe), "200"], capture_output=True, text=True, timeout=120)
                self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
                parts = run.stdout.split()
                self.assertGreaterEqual(len(parts), 2, run.stdout)
                outputs.append((passes or "none", parts[1]))
            checksums = {value for _, value in outputs}
            self.assertEqual(
                len(checksums), 1,
                f"kill switches changed the kernel result: {outputs}",
            )


if __name__ == "__main__":
    unittest.main()
