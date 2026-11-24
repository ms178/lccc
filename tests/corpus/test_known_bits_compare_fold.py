#!/usr/bin/env python3
"""A comparison the operands' known bits decide is a constant.

`(x & 7) < 8`, `(x | 8) >= 8`, `(x | 1) == 0`, `(x | INT_MIN) < 0`,
`(x >> 28) < 16`: GCC 16.2 and Clang 23 return the constant for each. LCCC
computed every one of them (`andl $7, %edi; cmpl $8, %edi; setb %al`): its
fact engine knew the bits and folded a comparison only behind a guard, and
only for equality.

`bit_idioms_facts::Context::truth` now decides an ordering from the least
and the greatest value the known bits allow, `fold` no longer asks for a
guard, and a select on a decided condition is the arm it chooses. The Rust
unit tests check the rule against every pair of values over a table of
known-bit states; this module pins what a program sees:

1. each decided function is a constant at `-O2`, `-O3` and `-Os` (the levels
   that run `bit_idioms`), for x86-64 and i686: no compare, test, setcc,
   cmov or conditional jump;
2. a comparison one value away from decided is still computed, and so is the
   AND of two words that are each non-zero and share no bit
   (`(x | 1) & (y | 2)`: the fold that used to answer "true" for it is the
   miscompile `tests/regression/bit_idioms_and_of_nonzero_words.c` records);
3. every function returns what `-O0` returns and what GCC and Clang return.
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
LCCC = Path(os.environ.get("LCCC", REPO / "target/fastbuild/lccc"))
GCC = os.environ.get("GCC16") or shutil.which("gcc-16") or shutil.which("gcc")
CLANG = os.environ.get("CLANG") or shutil.which("clang-23")

# name -> True when the function must be a constant.
FUNCTIONS = {
    "mask_below": True,
    "or_at_least": True,
    "sign_set": True,
    "or_above": True,
    "byte_above": True,
    "promoted_byte": True,
    "shifted_below": True,
    "or_is_zero": True,
    "or_is_zero_wide": True,
    "sign_clear": True,
    "masked_equal": True,
    "shifted_equal": True,
    "through_cast": True,
    "truncated": True,
    "select_on_word": True,
    "guarded_scan": True,
    "ffs_never_zero": True,
    # One value away from decided, or not decided at all.
    "mask_below_open": False,
    "or_above_open": False,
    "or_is_one": False,
    "sign_open": False,
    "words_share_no_bit": False,
    "select_on_open_word": False,
}

SOURCE = r"""
int mask_below(unsigned x) { return (x & 7) < 8; }
int or_at_least(unsigned x) { return (x | 8) >= 8; }
int sign_set(int x) { return (x | (int)0x80000000) < 0; }
int or_above(unsigned x) { return (x | 0x10) > 0xf; }
int byte_above(unsigned x) { return (x & 0xff) > 0x100; }
int promoted_byte(unsigned char x) { return (x | 0x80) < 0x80; }
int shifted_below(unsigned x) { return (x >> 28) < 16; }
int or_is_zero(unsigned x) { return (x | 1) == 0; }
int or_is_zero_wide(unsigned long long x) { return (x | 1) == 0; }
int sign_clear(int x) { return (x & 0x7fffffff) >= 0; }
int masked_equal(unsigned x) { return (x & 6) == 1; }
int shifted_equal(unsigned x) { return (x << 3 | 8) == 16; }
int through_cast(unsigned x) { return (int)(x | 1) == 0; }
int truncated(unsigned long long x) { return (unsigned)(x | 2) != 0; }
int select_on_word(unsigned x, int a, int b) { (void)b; return (x | 4) ? 7 : a; }
int guarded_scan(unsigned x) { unsigned y = x | 1; return y ? __builtin_ctz(y) : 32; }
int ffs_never_zero(unsigned x) { return __builtin_ffs((int)(x | 1)); }
int mask_below_open(unsigned x) { return (x & 7) < 7; }
int or_above_open(unsigned x) { return (x | 8) > 8; }
int or_is_one(unsigned x) { return (x | 1) == 1; }
int sign_open(int x) { return (x | 0x40000000) < 0; }
int words_share_no_bit(unsigned x, unsigned y) { return ((x | 1) & (y | 2)) != 0; }
int select_on_open_word(unsigned x, unsigned y) { return ((x | 1) & (y | 2)) ? 7 : 9; }
"""

DRIVER = r"""
#include <stdio.h>
int main(void) {
    unsigned long long h = 1469598103934665603ull;
    unsigned x = 0, y = 0;
    for (int i = 0; i < 4096; i++) {
        /* Every single bit, its complement, the small values in which the
           two words of the last functions share no bit, then a walk. */
        if (i < 32) x = 1u << i, y = 0;
        else if (i < 64) x = ~(1u << (i - 32)), y = 1u << (i - 32);
        else if (i < 128) x = (unsigned)(i - 64) & 7, y = (unsigned)(i - 64) >> 3;
        else x = x * 2654435761u + 0x9e3779b9u, y = y * 40503u + x;
        unsigned long long w = ((unsigned long long)x << 32) | y;
        int r[] = {
            mask_below(x), or_at_least(x), sign_set((int)x), or_above(x), byte_above(x),
            promoted_byte((unsigned char)x), shifted_below(x), or_is_zero(x), or_is_zero_wide(w),
            sign_clear((int)x), masked_equal(x), shifted_equal(x), through_cast(x), truncated(w),
            select_on_word(x, (int)y, 3), guarded_scan(x), ffs_never_zero(x), mask_below_open(x),
            or_above_open(x), or_is_one(x), sign_open((int)x), words_share_no_bit(x, y),
            select_on_open_word(x, y),
        };
        for (unsigned k = 0; k < sizeof r / sizeof r[0]; k++)
            h = (h ^ (unsigned long long)(unsigned)r[k]) * 1099511628211ull;
        if (i == 0 || i == 64 || i == 65 || i == 72 || i == 127) {
            for (unsigned k = 0; k < sizeof r / sizeof r[0]; k++) printf("%d ", r[k]);
            printf("\n");
        }
    }
    printf("%016llx\n", h);
    return 0;
}
"""

# What computing a condition looks like. `xor %eax, %eax` and `mov $1, %eax`
# are neither; a plain `and`/`or` of the operand is not a decision either,
# so the open functions are recognised by these alone.
DECISION = re.compile(
    r"^\s*(?:cmp|test|set[a-z]+|cmov[a-z]+|j(?!mp)[a-z]+|bt|sbb|adc|neg|bsf|bsr|tzcnt|lzcnt)[lqwb]?\b",
    re.M,
)


def function_bodies(assembly: str) -> dict[str, str]:
    """The text between each function's label and its `.size` directive."""
    bodies = {}
    for name in FUNCTIONS:
        match = re.search(
            rf"^{re.escape(name)}:\n(.*?)^\s*\.size\s+{re.escape(name)}\b", assembly, re.M | re.S
        )
        if match:
            bodies[name] = match.group(1)
    return bodies


@unittest.skipUnless(LCCC.exists(), f"no compiler at {LCCC}")
class KnownBitsCompareFold(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="known-bits-compare-")
        cls.dir = Path(cls.tmp.name)
        (cls.dir / "k.c").write_text(SOURCE)
        (cls.dir / "main.c").write_text(SOURCE + DRIVER)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def assembly(self, *flags: str) -> str:
        out = self.dir / "k.s"
        subprocess.run(
            [str(LCCC), *flags, "-S", str(self.dir / "k.c"), "-o", str(out)],
            check=True,
            capture_output=True,
            text=True,
        )
        return out.read_text()

    def run_program(self, compiler: str, *flags: str) -> str:
        exe = self.dir / "prog"
        subprocess.run(
            [compiler, *flags, str(self.dir / "main.c"), "-o", str(exe)],
            check=True,
            capture_output=True,
            text=True,
        )
        return subprocess.run([str(exe)], check=True, capture_output=True, text=True).stdout

    def test_a_decided_comparison_is_a_constant(self):
        for mode in ((), ("-m32",)):
            for level in ("-O2", "-O3", "-Os"):
                bodies = function_bodies(self.assembly(level, *mode))
                self.assertEqual(sorted(bodies), sorted(FUNCTIONS), (level, mode))
                for name, decided in FUNCTIONS.items():
                    with self.subTest(function=name, level=level, mode=mode):
                        decisions = DECISION.findall(bodies[name])
                        if decided:
                            self.assertEqual(decisions, [], bodies[name])
                        else:
                            self.assertNotEqual(decisions, [], bodies[name])

    def test_the_constants_are_what_the_comparisons_compute(self):
        reference = self.run_program(str(LCCC), "-O0")
        self.assertEqual(len(reference.splitlines()), 6)
        for level in ("-O1", "-O2", "-O3", "-Os"):
            with self.subTest(level=level):
                self.assertEqual(self.run_program(str(LCCC), level), reference)
        with self.subTest(target="x86-64-v3"):
            self.assertEqual(self.run_program(str(LCCC), "-O2", "-march=x86-64-v3"), reference)
        for other in (GCC, CLANG):
            if other:
                with self.subTest(compiler=other):
                    self.assertEqual(self.run_program(other, "-O2"), reference)


if __name__ == "__main__":
    unittest.main()
