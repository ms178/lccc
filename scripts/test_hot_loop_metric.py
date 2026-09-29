#!/usr/bin/env python3
"""Known-answer tests for `scripts/hot_loop_metric.py` loop identification.

The metric decides which compiler wins on a loop kernel, so it is tested
like a compiler pass: with hand-built control flow whose right answer is
known in advance, plus the real shape that broke it.

Each case is a synthetic function body (AT&T, ``.L*`` labels) written to a
temporary file and run through :func:`hot_loop_metric.analyse`.

1. ``scalar_byte_loop``   -- one loop, step 1, density is the body size.
2. ``packed_plus_remainder`` -- the vectorized shape; the packed body must win
   over the shorter scalar remainder (that inversion is the whole point of
   the steady-state policy).
3. ``nested_matmul``      -- the LCCC/GCC matrix-multiply shape: an inner
   packed loop inside a middle loop inside an outer loop, and a scalar
   remainder whose guard jumps *backwards in the listing* to the middle
   loop's latch.  The measured loop must be the inner packed body; the
   enclosing loops must be classified ``composite`` and never measured;
   the bogus "remainder guard -> latch" edge must not be a loop at all.
4. ``dominance_not_layout`` -- a branch to a textually earlier block that does
   not dominate it (the minimal form of case 3's trap): not a loop.
   ``backwards_not_backedge`` -- the same trap with a second loop entry, i.e.
   an irreducible cycle: no density rather than a guessed one.
5. ``irreducible``        -- a cycle with two entries: dominance finds no
   header, so the tool reports no density instead of picking a wrong one.
6. ``no_loop``            -- straight-line code: an error row, not a crash.

Exit status is non-zero on the first failed assertion, so this doubles as a
CI gate (`hot-loop-metric` in `scripts/ci_local.sh` / `.github/workflows/ci.yml`).
"""
from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import hot_loop_metric as H  # noqa: E402

# --- case 1: one scalar loop over bytes -------------------------------------
SCALAR = """
kernel:
    xorl %eax, %eax
.L1:
    movzbl (%rsi,%rax), %ecx
    addl %ecx, %edx
    addq $1, %rax
    cmpq %rdi, %rax
    jne .L1
    movl %edx, %eax
    ret
"""

# --- case 2: vectorized body plus a scalar remainder ------------------------
PACKED_REMAINDER = """
kernel:
    xorl %eax, %eax
.L2:
    cmpq %rdi, %rax
    jge .L5
.L3:
    vmovdqu (%rsi,%rax), %ymm0
    vpaddd (%rdx,%rax), %ymm0, %ymm0
    vmovdqu %ymm0, (%rdx,%rax)
    addq $32, %rax
    cmpq %rdi, %rax
    jl .L3
.L5:
    cmpq %rdi, %rax
    jge .L7
.L6:
    movl (%rsi,%rax), %ecx
    addl %ecx, (%rdx,%rax)
    addq $4, %rax
    cmpq %rdi, %rax
    jl .L6
.L7:
    ret
"""

# --- case 3: the real matmul shape (LCCC -O2 -march=x86-64-v3) --------------
# Outer i-loop (.L2) > middle k-loop (.L4) > inner packed j-loop (.L6) plus a
# scalar remainder (.L11) whose guard jumps backwards in the listing to the
# middle loop's latch (.L7).  .L7 does NOT dominate .L10, so that edge is not
# a backedge and must not produce a "loop" costing 24 insns per 2048 bytes.
NESTED_MATMUL = """
matmul:
    xorl %r12d, %r12d
.L1:
    cmpq $256, %r12
    jge .L12
.L2:
    xorl %esi, %esi
.L3:
    cmpq $256, %rsi
    jge .L8
.L4:
    vbroadcastsd (%r14), %ymm1
    xorl %r11d, %r11d
.L5:
    cmpq $2048, %r11
    jge .L9
.L6:
    vmovupd (%r8,%r11), %ymm0
    vfmadd231pd (%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, (%r8,%r11)
    vmovupd 32(%r8,%r11), %ymm0
    vfmadd231pd 32(%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, 32(%r8,%r11)
    vmovupd 64(%r8,%r11), %ymm0
    vfmadd231pd 64(%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, 64(%r8,%r11)
    vmovupd 96(%r8,%r11), %ymm0
    vfmadd231pd 96(%rdi,%r11), %ymm1, %ymm0
    vmovupd %ymm0, 96(%r8,%r11)
    addq $128, %r11
    cmpq $2048, %r11
    jl .L6
    jmp .L9
.L7:
    addq $1, %rsi
    leaq 2048(%rdi), %rdi
    leaq 8(%r14), %r14
    cmpq $256, %rsi
    jl .L4
.L8:
    addq $1, %r12
    leaq 2048(%r8), %r8
    leaq 2048(%r13), %r13
    cmpq $256, %r12
    jl .L2
    jmp .L12
.L9:
    movq %r11, %rax
    shrl $3, %eax
    movslq %eax, %r11
.L10:
    cmpq $256, %r11
    jge .L7
.L11:
    vmovsd (%r10), %xmm2
    vfmadd231sd (%rdx), %xmm3, %xmm2
    vmovsd %xmm2, (%r10)
    addq $1, %r11
    leaq 8(%r10), %r10
    leaq 8(%rdx), %rdx
    cmpq $256, %r11
    jl .L11
    jmp .L7
.L12:
    ret
"""

# --- case 4: a backwards-in-the-listing jump that is not a backedge ---------
DOMINANCE_NOT_LAYOUT = """
kernel:
    xorl %eax, %eax
.L1:
    movl (%rsi,%rax), %ecx
    addl %ecx, %edx
    addq $4, %rax
    cmpq %rdi, %rax
    jge .L2
    jmp .L1
.L2:
    cmpq $8, %rax
    jl .L1
    movl %edx, %eax
    ret
"""

# --- case 4b: a backwards jump whose target does not dominate ---------------
# The listing-order trap in its minimal form: `jl .L2` targets a label that
# appears EARLIER in the file, which the pre-fix "branch to an earlier label"
# rule called a backedge.  .L2 does not dominate .L3 (which is also reached
# from .L1), so there is no loop header at all.
BACKWARDS_NOT_BACKEDGE = """
kernel:
    cmpl $0, %edi
    je .L2
.L1:
    movl $1, %eax
    jmp .L3
.L2:
    movl $2, %eax
    jmp .L3
.L3:
    cmpq %rdi, %rsi
    jl .L2
    ret
"""

# --- case 5: irreducible cycle (two entries, no dominating header) ----------
IRREDUCIBLE = """
kernel:
    xorl %ecx, %ecx
    cmpl $0, %edi
    je .L2
    jmp .L1
.L1:
    addl $1, %ecx
    addq $4, %rsi
.L2:
    addl $1, %ecx
    addq $4, %rsi
    cmpl $16, %ecx
    jl .L1
    movl %ecx, %eax
    ret
"""

# --- case 6: straight line --------------------------------------------------
NO_LOOP = """
kernel:
    movl (%rsi), %eax
    addl (%rdx), %eax
    ret
"""


class HotLoopMetricTest(unittest.TestCase):
    def analyse(self, source: str, symbol: str = "kernel") -> dict:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "case.s"
            path.write_text(source)
            return H.analyse(path, symbol)

    def loop_by_label(self, result: dict, label: str) -> dict:
        for row in result["loops"]:
            if row["loop_label"] == label:
                return row
        self.fail(f"loop {label} not found in {[r['loop_label'] for r in result['loops']]}")

    def test_scalar_byte_loop(self) -> None:
        r = self.analyse(SCALAR)
        self.assertNotIn("error", r)
        self.assertEqual(r["loop_label"], ".L1")
        self.assertEqual(r["bytes_per_trip"], 1)
        self.assertEqual(r["insns"], 5)
        self.assertAlmostEqual(r["insns_per_byte"], 5.0)

    def test_packed_body_beats_shorter_remainder(self) -> None:
        r = self.analyse(PACKED_REMAINDER)
        self.assertNotIn("error", r)
        self.assertEqual(r["loop_label"], ".L3")
        self.assertEqual(r["bytes_per_trip"], 32)
        self.assertEqual(r["insns"], 6)
        self.assertAlmostEqual(r["insns_per_byte"], 6 / 32)
        # The remainder is still reported, just not chosen.
        rem = self.loop_by_label(r, ".L6")
        self.assertEqual(rem["bytes_per_trip"], 4)
        self.assertFalse(rem["composite"])

    def test_nested_matmul_measures_the_inner_packed_body(self) -> None:
        r = self.analyse(NESTED_MATMUL, "matmul")
        self.assertNotIn("error", r)
        # The inner packed loop, not the middle/outer loop, is the steady
        # state: 12 vector ops + 3 loop-control ops per 128 bytes.
        self.assertEqual(r["loop_label"], ".L6")
        self.assertEqual(r["bytes_per_trip"], 128)
        self.assertEqual(r["insns"], 15)
        self.assertAlmostEqual(r["insns_per_byte"], 15 / 128)
        # The enclosing loops are recognised and explicitly not measured.
        for label in (".L4", ".L2"):
            outer = self.loop_by_label(r, label)
            self.assertTrue(outer["composite"], f"{label} must be composite")
        # The remainder guard's backwards jump to the latch is not a loop.
        self.assertNotIn(".L10", [row["loop_label"] for row in r["loops"]])
        self.assertNotIn(".L7", [row["loop_label"] for row in r["loops"]])

    def test_composite_loops_are_never_measured(self) -> None:
        r = self.analyse(NESTED_MATMUL, "matmul")
        # Regression: the pre-fix tool picked a composite cycle (the middle
        # k-loop, whose body contains the packed inner loop) and charged its
        # 24 static instructions against a 2048-byte trip, i.e. it reported
        # 0.0117 insn/byte for a kernel whose real cost is 0.1172.
        self.assertFalse(r["composite"], "the measured loop must be innermost")
        wide = [row for row in r["loops"] if row["composite"]]
        self.assertTrue(wide, "the enclosing loops should be reported as composite")
        for row in wide:
            # A loop that encloses another loop has no static per-trip step,
            # so it must carry NO density at all - that omission is the
            # regression (the pre-fix tool reported 0.0117 insn/byte here,
            # and charging the composite body's 2177 recovered "bytes per
            # trip" would be the same lie in a different unit).
            self.assertIsNone(row["insns_per_byte"])
            self.assertEqual(row["bytes_per_trip"], 0)
            self.assertNotEqual(row["loop_label"], r["loop_label"])
        # The chosen loop is the packed inner body: 128 bytes per trip.
        self.assertEqual(r["bytes_per_trip"], 128)
        self.assertAlmostEqual(r["insns_per_byte"], 15 / 128, places=9)

    def test_dominance_not_text_layout(self) -> None:
        r = self.analyse(DOMINANCE_NOT_LAYOUT)
        self.assertNotIn("error", r)
        # One loop (the `jmp .L1` head), and the listing-order jump back to
        # .L1 from .L2 is a genuine backedge -- .L1 dominates .L2 here.  The
        # point of the case is that the loop is found ONCE, per header.
        self.assertEqual([row["loop_label"] for row in r["loops"]], [".L1"])

    def test_irreducible_cycle_reports_no_density(self) -> None:
        r = self.analyse(IRREDUCIBLE)
        # No single dominating header -> no natural loop -> nothing measured.
        # The tool must say WHY rather than fall back on "the tightest cycle",
        # which is how a wrong density used to reach the evidence table.
        self.assertEqual(r["loops"], [])
        self.assertIn("error", r)
        self.assertIn("irreducible", r["error"])

    def test_backwards_jump_without_dominance_is_not_a_loop(self) -> None:
        r = self.analyse(BACKWARDS_NOT_BACKEDGE)
        # `jl .L2` jumps to a textually earlier label, but .L2 does not
        # dominate the block that jumps, so this is a cycle with two entries
        # (irreducible), not a natural loop.
        self.assertEqual(r["loops"], [])
        self.assertIn("irreducible", r["error"])
        self.assertIsNone(r.get("insns_per_byte"))

    def test_straight_line_is_an_error_not_a_crash(self) -> None:
        r = self.analyse(NO_LOOP)
        self.assertIn("error", r)
        self.assertIn("no loop found", r["error"])

    def test_real_oracle_assembly_if_present(self) -> None:
        """Optional: the captured matmul comparison (lccc vs gcc 16.2).

        Skipped when the artifacts are absent so the gate stays offline; run
        `scripts/codegen_oracle.py` + `scripts/oracle_asm.py` to refresh them.
        """
        root = Path(__file__).resolve().parents[1]
        lccc = root / "tests" / "asm" / "hot-loop" / "matmul.lccc.s"
        gcc = root / "tests" / "asm" / "hot-loop" / "matmul.gcc162.s"
        if not (lccc.exists() and gcc.exists()):
            self.skipTest("captured matmul assembly not present")
        lr = H.analyse(lccc, "matmul")
        gr = H.analyse(gcc, "matmul")
        self.assertNotIn("error", lr)
        self.assertNotIn("error", gr)
        # Both measure their own packed inner loop; LCCC's is 4x unrolled.
        self.assertEqual(lr["bytes_per_trip"], 128)
        self.assertEqual(gr["bytes_per_trip"], 32)
        self.assertLess(lr["insns_per_byte"], gr["insns_per_byte"])


if __name__ == "__main__":
    sys.exit(0 if unittest.main(exit=False).result.wasSuccessful() else 1)
