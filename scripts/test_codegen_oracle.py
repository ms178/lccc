#!/usr/bin/env python3
"""Known-answer tests for `scripts/codegen_oracle.py`'s comparability guard.

WHY THIS EXISTS
---------------
`--rank` orders work by "how many instructions is this function behind the best
oracle". That number is only a code-generation gap if both compilers put the
SAME work in the function. When one of them declines to inline, its body is
smaller and the callee it moved out of line is charged to nobody, so the
"winner" is an inlining decision wearing a codegen costume.

This was not hypothetical. On the 51-program benchmark corpus at
`-O2 -march=x86-64-v3`, 16 of the 77 "behind" rows compared functions with
different call counts, and they accounted for 781 of the 1899-instruction
headline deficit -- 41 %, including both of the top two rows. The #1 row,
`zlib_ng_adler32::main` at "204 behind icc=73", was ICC leaving
`zlib_ng_adler32_c` out of line as two specialised copies (`..0`, `..1`) that
the single-function view never measured. Whole translation unit, ICC is 43 %
LARGER than LCCC over the six files that table put at the top.

Full write-up: `engineering/evidence/ORACLE-METRIC-1/README.md`.

WHAT IS TESTED
--------------
1. `_stats` counts call sites on x86, AArch64 and RISC-V.
2. `_stats` does NOT count plain jumps as calls. A tail jump to an out-of-line
   copy is precisely the shape that fakes a smaller function, and counting it
   as a call would hide the very rows this guard exists to expose.
3. `_rank_markdown` marks a row non-comparable exactly when the call counts
   differ, and leaves a genuine gap (equal call counts) marked comparable.
4. `_print_rank` says so on stdout, with the count and the pointer to
   `--all-functions`.

These are pure-logic tests: hand-built assembly and hand-built rows, no
Compiler Explorer traffic, so they run in the fast CI tier.

Exit status is non-zero on the first failed assertion.
"""
from __future__ import annotations

import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import codegen_oracle as CG  # noqa: E402


def _rows(*specs):
    """Build RankRow tuples: (gap, bench, fname, local_stats, best_name, per)."""
    out = []
    for gap, bench, fname, lcalls, bcalls, linsns, binsns in specs:
        local = CG.AsmStats(instructions=linsns, calls=lcalls)
        best = CG.AsmStats(instructions=binsns, calls=bcalls)
        out.append((gap, bench, fname, local, "gcc16.2", {"gcc16.2": best}))
    return out


class CallCounting(unittest.TestCase):
    def test_x86_calls_are_counted(self):
        body = [
            "main:",
            "    pushq %rbp",
            "    call printf",
            "    call zlib_ng_adler32_c..0",
            "    call *%rax",
            "    ret",
        ]
        self.assertEqual(CG._stats(body, "x86").calls, 3)

    def test_aarch64_bl_and_blr_are_counted(self):
        body = ["f:", "    stp x29, x30, [sp, -16]!", "    bl printf",
                "    blr x8", "    ret"]
        self.assertEqual(CG._stats(body, "aarch64").calls, 2)

    def test_riscv_call_forms_are_counted(self):
        body = ["f:", "    addi sp, sp, -16", "    call printf",
                "    jal ra, g", "    jalr ra, t0", "    ret"]
        self.assertEqual(CG._stats(body, "riscv64").calls, 3)

    def test_plain_jumps_are_NOT_calls(self):
        """The important negative case.

        A compiler that specialises a callee out of line often reaches it with a
        tail jump, not a call. Counting `jmp`/`b`/`j` as a call would mark those
        rows comparable and hide exactly the artifact this guard exists to
        expose, so jumps are deliberately excluded.
        """
        for arch, jumps in (("x86", ["jmp .L2", "je .L3", "jne printf"]),
                            ("aarch64", ["b .L2", "b.eq .L3", "b printf"]),
                            ("riscv64", ["j .L2", "beq a0, a1, .L3"])):
            body = ["f:"] + [f"    {j}" for j in jumps] + ["    ret"]
            self.assertEqual(CG._stats(body, arch).calls, 0,
                             f"{arch}: a jump was counted as a call")

    def test_directives_and_labels_are_not_instructions_or_calls(self):
        body = ["main:", "    .cfi_startproc", "    .p2align 4",
                "    call printf", "    .size main, .-main"]
        st = CG._stats(body, "x86")
        self.assertEqual(st.calls, 1)
        self.assertEqual(st.instructions, 1)


class MarkdownComparability(unittest.TestCase):
    def _render(self, rows):
        with tempfile.TemporaryDirectory() as td:
            p = Path(td) / "r.md"
            CG._rank_markdown(rows, False, p, "-O2")
            return p.read_text()

    def test_differing_call_counts_are_flagged_and_real_gaps_are_not(self):
        rows = _rows(
            # the real shape: ICC's stub `main` with 4 calls vs our inlined one
            (204, "zlib_ng_adler32", "main", 1, 4, 277, 73),
            # a genuine gap: same call count, so the bodies are comparable
            (90, "moving_stats", "main", 1, 1, 221, 131),
        )
        md = self._render(rows)
        self.assertIn("| LCCC calls | Best calls | Comparable |", md)
        # adler32 row: 1 vs 4 -> not comparable
        adler = next(l for l in md.split("\n") if "zlib_ng_adler32" in l)
        self.assertIn("| 1 | 4 | **no** |", adler)
        # moving_stats row: 1 vs 1 -> comparable, and must NOT be flagged
        moving = next(l for l in md.split("\n") if "moving_stats" in l)
        self.assertIn("| 1 | 1 | yes |", moving)
        self.assertNotIn("**no**", moving)

    def test_summary_reports_the_share_of_the_deficit(self):
        rows = _rows((204, "zlib_ng_adler32", "main", 1, 4, 277, 73),
                     (188, "nbody", "main", 2, 4, 306, 118),
                     (90, "moving_stats", "main", 1, 1, 221, 131))
        md = self._render(rows)
        # 2 of 3 rows, 392 of the 482 total deficit
        self.assertIn("## 2 rows are not comparable (392 of 482 of the deficit)", md)
        self.assertIn("--all-functions", md)
        self.assertIn("ORACLE-METRIC-1", md)

    def test_no_flagged_rows_means_no_warning_section(self):
        md = self._render(_rows((90, "moving_stats", "main", 1, 1, 221, 131)))
        self.assertNotIn("not comparable", md)

    def test_rows_we_are_ahead_on_are_never_flagged(self):
        """A negative gap is a win; whether the call counts match is moot and
        flagging it would train readers to ignore the marker."""
        md = self._render(_rows((-40, "chacha20_block", "core", 3, 0, 63, 103)))
        self.assertNotIn("not comparable", md)


class StdoutWarning(unittest.TestCase):
    def test_print_rank_warns_and_points_at_all_functions(self):
        rows = _rows((204, "zlib_ng_adler32", "main", 1, 4, 277, 73),
                     (90, "moving_stats", "main", 1, 1, 221, 131))
        buf = io.StringIO()
        with redirect_stdout(buf):
            CG._print_rank(rows, False, {})
        out = buf.getvalue()
        self.assertIn("<-- CALLS DIFFER, gap not comparable", out)
        self.assertIn("WARNING: 1 of the 2 'behind' rows", out)
        self.assertIn("zlib_ng_adler32:main (lccc 1 calls vs gcc16.2 4)", out)
        self.assertIn("--all-functions", out)
        # the comparable row must carry no marker
        moving = next(l for l in out.split("\n") if "moving_stats" in l)
        self.assertNotIn("CALLS DIFFER", moving)

    def test_clean_run_prints_no_warning(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            CG._print_rank(_rows((90, "moving_stats", "main", 1, 1, 221, 131)),
                           False, {})
        self.assertNotIn("WARNING", buf.getvalue())


if __name__ == "__main__":
    sys.exit(0 if unittest.main(exit=False).result.wasSuccessful() else 1)
