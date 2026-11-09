#!/usr/bin/env python3
"""Self-tests for check_aligned_vector_reload.

The gate's own scanner is a small analyser, so it needs the same treatment as
the compiler pass: every refusal must be exercised, and the historical defect
must be detected.  A scanner that reports nothing on any input would make the
gate pass forever.
"""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "avr_gate", HERE / "check_aligned_vector_reload.py"
)
assert SPEC is not None and SPEC.loader is not None
gate = importlib.util.module_from_spec(SPEC)
sys.modules["avr_gate"] = gate
SPEC.loader.exec_module(gate)

DEFECT = """main:
    leaq -193(%rbp), %rax
    andq $-32, %rax
    vmovdqu %ymm0, (%rax)
    leaq -193(%rbp), %rax
    andq $-32, %rax
    vmovups (%rax), %ymm3
"""

FIXED = DEFECT.replace("vmovups (%rax), %ymm3", "vmovups %ymm0, %ymm3")


class DetectionTests(unittest.TestCase):
    def test_the_historical_defect_is_detected(self):
        problems = gate.find_defects(DEFECT)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("%ymm0", problems[0])
        self.assertIn("(-193, 32)", problems[0])

    def test_the_fixed_shape_is_clean(self):
        self.assertEqual(gate.find_defects(FIXED), [])

    def test_a_plain_frame_address_round_trip_is_detected(self):
        asm = """f:
    leaq -64(%rbp), %rcx
    vmovaps %xmm1, (%rcx)
    vmovaps (%rcx), %xmm2
"""
        self.assertEqual(len(gate.find_defects(asm)), 1)

    def test_zmm_round_trip_is_detected(self):
        asm = """f:
    leaq -128(%rbp), %rax
    andq $-64, %rax
    vmovdqu64 %zmm2, (%rax)
    vmovdqa64 (%rax), %zmm5
"""
        self.assertEqual(len(gate.find_defects(asm)), 1)

    def test_a_narrower_reload_is_also_a_defect(self):
        asm = DEFECT.replace("vmovups (%rax), %ymm3", "vmovups (%rax), %xmm3")
        self.assertEqual(len(gate.find_defects(asm)), 1)

    def test_two_reloads_of_one_store_are_two_defects(self):
        asm = DEFECT + "    vmovups (%rax), %ymm4\n"
        self.assertEqual(len(gate.find_defects(asm)), 2)

    def test_a_zero_displacement_lea_is_a_key(self):
        asm = """f:
    leaq (%rbp), %rdx
    vmovdqu %ymm4, (%rdx)
    vmovdqu (%rdx), %ymm5
"""
        self.assertEqual(len(gate.find_defects(asm)), 1)


class RefusalTests(unittest.TestCase):
    def test_a_call_between_store_and_load_is_not_a_defect(self):
        asm = DEFECT.replace(
            "    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
            "    call f1\n    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
        )
        self.assertEqual(gate.find_defects(asm), [])

    def test_a_source_register_rewrite_is_not_a_defect(self):
        asm = DEFECT.replace(
            "    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
            "    vpaddd %ymm1, %ymm2, %ymm0\n"
            "    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
        )
        self.assertEqual(gate.find_defects(asm), [])

    def test_a_different_address_is_not_a_defect(self):
        asm = DEFECT.replace(
            "leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
            "leaq -129(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
        )
        self.assertEqual(gate.find_defects(asm), [])

    def test_a_different_alignment_is_not_a_defect(self):
        asm = DEFECT.replace("andq $-32, %rax\n    vmovups", "andq $-16, %rax\n    vmovups")
        self.assertEqual(gate.find_defects(asm), [])

    def test_a_wider_load_is_not_a_defect(self):
        asm = DEFECT.replace("vmovups (%rax), %ymm3", "vmovdqu64 (%rax), %zmm3")
        self.assertEqual(gate.find_defects(asm), [])

    def test_an_encoding_class_change_is_not_a_defect(self):
        asm = DEFECT.replace("vmovdqu %ymm0, (%rax)", "movdqu %xmm0, (%rax)")
        asm = asm.replace("vmovups (%rax), %ymm3", "vmovups (%rax), %xmm3")
        self.assertEqual(gate.find_defects(asm), [])

    def test_a_label_between_them_is_not_a_defect(self):
        asm = DEFECT.replace("    vmovups (%rax), %ymm3", ".L9:\n    vmovups (%rax), %ymm3")
        self.assertEqual(gate.find_defects(asm), [])

    def test_an_unknown_store_between_them_is_not_a_defect(self):
        asm = DEFECT.replace(
            "    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
            "    movq %rdx, (%rdi)\n"
            "    leaq -193(%rbp), %rax\n    andq $-32, %rax\n    vmovups",
        )
        self.assertEqual(gate.find_defects(asm), [])

    def test_a_reload_into_the_source_ends_the_mapping(self):
        asm = DEFECT.replace("vmovups (%rax), %ymm3", "vmovups (%rax), %ymm0")
        asm += "    vmovups (%rax), %ymm4\n"
        self.assertEqual(len(gate.find_defects(asm)), 1)

    def test_partial_and_non_temporal_moves_are_ignored(self):
        for mnem in ("movsd", "movss", "vmovntdq", "vmovdqu8"):
            asm = (
                "f:\n    leaq -64(%rbp), %rax\n"
                f"    {mnem} %xmm0, (%rax)\n"
                f"    {mnem} (%rax), %xmm3\n"
            )
            self.assertEqual(gate.find_defects(asm), [], mnem)

    def test_an_indexed_operand_is_ignored(self):
        asm = (
            "f:\n    leaq -64(%rbp), %rax\n"
            "    vmovdqu %ymm0, (%rax,%rbx,4)\n"
            "    vmovups (%rax,%rbx,4), %ymm3\n"
        )
        self.assertEqual(gate.find_defects(asm), [])

    def test_a_displaced_operand_is_ignored(self):
        asm = (
            "f:\n    leaq -64(%rbp), %rax\n"
            "    vmovdqu %ymm0, 8(%rax)\n"
            "    vmovups 8(%rax), %ymm3\n"
        )
        self.assertEqual(gate.find_defects(asm), [])

    def test_an_and_without_a_lea_is_not_an_address(self):
        asm = (
            "f:\n    andq $-32, %rax\n"
            "    vmovdqu %ymm0, (%rax)\n"
            "    vmovups (%rax), %ymm3\n"
        )
        self.assertEqual(gate.find_defects(asm), [])

    def test_rsp_and_rbp_are_never_key_registers(self):
        for reg in ("%rsp", "%rbp"):
            asm = (
                "f:\n    leaq -64(%rbp), " + reg + "\n"
                f"    vmovdqu %ymm0, ({reg})\n"
                f"    vmovups ({reg}), %ymm3\n"
            )
            self.assertEqual(gate.find_defects(asm), [], reg)


class ScannerTests(unittest.TestCase):
    def test_width_is_taken_from_the_register_not_the_mnemonic(self):
        self.assertEqual(gate.width("%xmm3"), 16)
        self.assertEqual(gate.width("%ymm3"), 32)
        self.assertEqual(gate.width("%zmm3"), 64)

    def test_index_ignores_the_width_prefix(self):
        for reg in ("%xmm12", "%ymm12", "%zmm12"):
            self.assertEqual(gate.index(reg), 12)

    def test_empty_input_is_clean_not_a_crash(self):
        self.assertEqual(gate.find_defects(""), [])

    def test_comments_and_directives_do_not_break_the_scan(self):
        asm = DEFECT.replace(
            "    vmovups (%rax), %ymm3",
            "    # LCCC_CALL_FP 2\n    .cfi_remember_state\n    vmovups (%rax), %ymm3",
        )
        self.assertEqual(len(gate.find_defects(asm)), 1)

    def test_both_reproducers_exist(self):
        self.assertIn("noinline", gate.REPRO.read_text())
        self.assertIn("a * b + a", gate.REPRO.read_text())
        diff = gate.DIFF.read_text()
        for kernel in ("k_call_barrier", "k_alias_write", "k_volatile", "k_narrow_read"):
            self.assertIn(kernel, diff)


if __name__ == "__main__":
    unittest.main(verbosity=2)
