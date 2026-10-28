#!/usr/bin/env python3
"""The byte-IV divide recognizer accepts every legal lowering and nothing else.

Two assembly shape gates (check_matmul_negative_bound.sh and
check_vector_remainder_codegen.sh) now share
``tests/regression/lib/byte_iv_divide.py``. A shared recognizer that is too
permissive silently disables both gates, so its accept/reject boundary is
tested here rather than only through the gates' own verdicts -- the gates can
only observe the one shape their fixture happens to emit.

The "passing" sample is not invented: it is the `matmul_vector_loop` body the
compiler emitted for tests/regression/vectorize_matmul_negative_bound.c under
`-O2 -march=x86-64-v3` (I32 bound, AVX2 arm), which is the exact output that
made both gates fail against a correct compiler.
"""

import sys
import unittest
from pathlib import Path

LIB = Path(__file__).resolve().parent.parent / "regression" / "lib"
sys.path.insert(0, str(LIB))

from byte_iv_divide import divides_byte_iv, narrows_after_divide  # noqa: E402

# Verbatim tail of the emitted epilogue: byte IV in %rsi, element index in
# %rdi, the I32 narrowing re-extended by movslq after the divide.
BMI2_EPILOGUE = """\
.LBB4:
\tleaq dest(%rip), %r10
\trorxq $3, %rsi, %rsi
\tmovslq %esi, %rdi
\tmovslq %ebx, %rsi
"""

# The same shape without BMI2 (`-march=x86-64`, the two-wide arm).
LEGACY_EPILOGUE = """\
.LBB4:
\tleaq dest(%rip), %r10
\tshrq $3, %rsi
\tmovslq %esi, %rdi
"""


class AcceptsEveryLegalLowering(unittest.TestCase):
    def test_bmi2_rotate_is_a_64_bit_divide(self):
        self.assertTrue(divides_byte_iv(BMI2_EPILOGUE, 3, require_64bit=True))
        self.assertTrue(divides_byte_iv(BMI2_EPILOGUE, 3))

    def test_legacy_shift_is_a_64_bit_divide(self):
        self.assertTrue(divides_byte_iv(LEGACY_EPILOGUE, 3, require_64bit=True))

    def test_both_forms_satisfy_the_narrowing_requirement(self):
        self.assertTrue(narrows_after_divide(BMI2_EPILOGUE, 3))
        self.assertTrue(narrows_after_divide(LEGACY_EPILOGUE, 3))

    def test_high_registers_map_to_their_32_bit_alias(self):
        body = "\trorxq $3, %r11, %r11\n\tmovslq %r11d, %r9\n"
        self.assertTrue(narrows_after_divide(body, 3))

    def test_a_memory_destination_divide_still_counts(self):
        # The gates predate this module and accepted any destination form; a
        # strictly line-shaped pattern would have started rejecting the spilling
        # form (`shrl $2, -4(%rbp)`) that the reduction kernels emit.
        self.assertTrue(divides_byte_iv("\tshrl $2, -4(%rbp)\n", 2))
        self.assertTrue(divides_byte_iv("\tshrq $3, 8(%rsp)\n", 3, require_64bit=True))

    def test_narrow_leniency_is_still_available_where_gates_need_it(self):
        # The remainder gate predates the 64-bit requirement and passes
        # require_64bit=False; a 32-bit spelling must keep satisfying it.
        body = "\tshrl $2, %eax\n"
        self.assertTrue(divides_byte_iv(body, 2))
        self.assertFalse(divides_byte_iv(body, 2, require_64bit=True))


class RejectsEveryIllegalShape(unittest.TestCase):
    def test_32_bit_divide_never_satisfies_the_64_bit_requirement(self):
        for body in ("\tshrl $3, %esi\n", "\trorxl $3, %esi, %esi\n"):
            with self.subTest(body=body):
                self.assertFalse(divides_byte_iv(body, 3, require_64bit=True))
                self.assertFalse(narrows_after_divide(body, 3))

    def test_a_longer_immediate_is_not_the_same_amount(self):
        # `$31` starts with `3`; the old gates' `\$3` prefix match accepted it.
        body = "\tshrq $31, %rsi\n\tmovslq %esi, %rdi\n"
        self.assertFalse(divides_byte_iv(body, 3, require_64bit=True))
        self.assertFalse(narrows_after_divide(body, 3))
        self.assertTrue(divides_byte_iv(body, 31, require_64bit=True))

    def test_narrowing_a_different_register_is_rejected(self):
        body = "\trorxq $3, %rsi, %rsi\n\tmovslq %ebx, %rdi\n"
        self.assertTrue(divides_byte_iv(body, 3, require_64bit=True))
        self.assertFalse(narrows_after_divide(body, 3))

    def test_narrowing_before_the_divide_is_rejected(self):
        body = "\tmovslq %esi, %rdi\n\trorxq $3, %rdi, %rdi\n"
        self.assertFalse(narrows_after_divide(body, 3))

    def test_missing_divide_is_rejected(self):
        body = "\tmovq %rsi, %rdi\n\taddq $8, %rdi\n"
        self.assertFalse(divides_byte_iv(body, 3, require_64bit=True))
        self.assertFalse(narrows_after_divide(body, 3))


if __name__ == "__main__":
    unittest.main()
