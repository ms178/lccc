#!/usr/bin/env python3
"""Unit tests for ci-codegen-gate.py's stackmem accounting.

The gate must count `N(%rbp)` as stack traffic only when rbp is the frame
pointer. In FPO functions rbp is a GPR (often homing a pointer parameter),
and `N(%rbp)` there is a heap/data dereference. These tests pin both
directions: the exclusion fires on proven GPR writes, and never on a true
frame-pointer function (so it cannot mask a real spill).
"""

import importlib.util
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).parent.resolve()


def load_gate():
    spec = importlib.util.spec_from_file_location(
        "ci_codegen_gate", HERE / "ci-codegen-gate.py"
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


gate = load_gate()


def func_asm(body: str, name: str = "f") -> str:
    return (
        f'\t.globl {name}\n\t.type {name}, @function\n{name}:\n{body}'
        f'\t.size {name}, .-{name}\n'
    )


class RbpAccountingTests(unittest.TestCase):
    def test_fpo_rbp_deref_excluded(self):
        asm = func_asm(
            "\tpushq %rbx\n"
            "\tsubq $24, %rsp\n"
            "\tmovq %rsi, %rbp\n"
            "\tmovq 8(%rbp), %r10\n"
            "\tmovq %rax, 8(%rsp)\n"
            "\tmovq 8(%rsp), %rax\n"
            "\taddq $24, %rsp\n"
            "\tret\n"
        )
        m = gate.function_body_metrics(asm)
        # Only the two 8(%rsp) slot refs count; the argv deref does not.
        self.assertEqual(m["stackmem"], 2)

    def test_frame_pointer_rbp_slot_counted(self):
        asm = func_asm(
            "\tpushq %rbp\n"
            "\tmovq %rsp, %rbp\n"
            "\tmovq %rdi, -8(%rbp)\n"
            "\tmovq -8(%rbp), %rax\n"
            "\tpopq %rbp\n"
            "\tret\n"
        )
        m = gate.function_body_metrics(asm)
        self.assertEqual(m["stackmem"], 2)

    def test_setup_mov_alone_proves_nothing(self):
        # `movq %rsp,%rbp` is the frame-pointer setup, not a GPR write: an
        # rbp slot ref in such a function must still count.
        asm = func_asm(
            "\tpushq %rbp\n"
            "\tmovq %rsp, %rbp\n"
            "\tmovq -16(%rbp), %rax\n"
            "\tpopq %rbp\n"
            "\tret\n"
        )
        m = gate.function_body_metrics(asm)
        self.assertEqual(m["stackmem"], 1)

    def test_per_function_isolation(self):
        fpo = func_asm(
            "\tmovq %rsi, %rbp\n"
            "\tmovq 8(%rbp), %r10\n"
            "\tret\n",
            name="fpo_fn",
        )
        framed = func_asm(
            "\tpushq %rbp\n"
            "\tmovq %rsp, %rbp\n"
            "\tmovq -8(%rbp), %rax\n"
            "\tpopq %rbp\n"
            "\tret\n",
            name="framed_fn",
        )
        m = gate.function_body_metrics(fpo + framed)
        # The FPO deref is excluded; the framed slot ref counts.
        self.assertEqual(m["stackmem"], 1)

    def test_lea_into_rbp_proves_gpr(self):
        asm = func_asm(
            "\tsubq $16, %rsp\n"
            "\tleaq 8(%rsp), %rbp\n"
            "\tmovq (%rbp), %rax\n"
            "\tmovq %rax, (%rsp)\n"
            "\taddq $16, %rsp\n"
            "\tret\n"
        )
        m = gate.function_body_metrics(asm)
        # The LEA line's own 8(%rsp) (alloca address) and the final store
        # count; only the (%rbp) deref is excluded.
        self.assertEqual(m["stackmem"], 2)

    def test_cmp_against_rbp_proves_nothing(self):
        # A comparison READS rbp; only pure writes prove GPR use. (A framed
        # function comparing against its own frame address must keep counting
        # its slot refs.)
        asm = func_asm(
            "\tpushq %rbp\n"
            "\tmovq %rsp, %rbp\n"
            "\tcmpq %rax, %rbp\n"
            "\tmovq -8(%rbp), %rax\n"
            "\tpopq %rbp\n"
            "\tret\n"
        )
        m = gate.function_body_metrics(asm)
        self.assertEqual(m["stackmem"], 1)


if __name__ == "__main__":
    result = unittest.main(exit=False, verbosity=2)
    sys.exit(0 if result.result.wasSuccessful() else 1)
