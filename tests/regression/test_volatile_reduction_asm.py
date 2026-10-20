#!/usr/bin/env python3
"""These tests must run identically with and without PYTHONOPTIMIZE."""

import unittest
from check_volatile_reduction_asm import accesses, check, ir_accesses


def assembly(count):
    return (
        "tls_product:\n"
        + "movq %fs:volatile_reduction_slots@TPOFF+8, %rax\n" * count
        + ".size tls_product, .-tls_product\n"
    )


class MechanismTests(unittest.TestCase):
    def test_valid(self):
        for count in (1, 5):
            check(assembly(count), count)

    def test_wrong_counts(self):
        for count in (0, 1, 4, 6):
            with self.subTest(count=count), self.assertRaises(ValueError):
                check(assembly(count), 5)

    def test_boundaries(self):
        for text in (
            "",
            assembly(5) * 2,
            assembly(5).replace(".size", ".wrong"),
            assembly(5).replace("tls_product:", "other_tls_product:"),
        ):
            with self.subTest(text=text), self.assertRaises(ValueError):
                accesses(text)

    def test_other_functions_and_offsets_do_not_count(self):
        self.assertEqual(
            accesses(assembly(1) + "other:\n" + assembly(4).split("\n", 1)[1]), 1
        )
        with self.assertRaises(ValueError):
            check(assembly(5).replace("TPOFF+8", "TPOFF+80"), 5)

    def test_comments_and_data_are_not_accesses(self):
        text = assembly(5).replace("movq %fs:", "# movq %fs:")
        with self.assertRaises(ValueError):
            check(text, 5)
        text = assembly(5).replace("movq %fs:", '.ascii "%fs:')
        with self.assertRaises(ValueError):
            check(text, 5)

    def test_ir_checkpoint(self):
        text = "==== IR after iter=0 volatile_reduce ====\n--- function add32 (7 blocks, 30 instrs) ---\n  Load { volatile: true }\n==== IR next ====\n  Load { volatile: true }\n"
        self.assertEqual(ir_accesses(text, "add32"), 1)
        for invalid in (
            "",
            text * 2,
            text.replace("function add32 ", "function not_add32 "),
        ):
            with self.assertRaises(ValueError):
                ir_accesses(invalid, "add32")


if __name__ == "__main__":
    unittest.main()
