#!/usr/bin/env python3
"""A bit scan whose answer the operand's known bits fix is that number.

`__builtin_ctz(x | 1)` is 0 whatever `x` is, `__builtin_clz(x | 0x80000000)`
is 0, `__builtin_ctz(x << 3 | 8)` is 3, and behind `if ((x & 7) == 4)` or
`if (x >> 28 == 1)` the scan of `x` is 2 or 3. LCCC kept the scan in all of
them (`orq $1, %rax; tzcntl %eax, %eax`); Clang 23 folds every one, GCC 16.2
folds the leading-zero and the guarded forms and keeps `rep bsf` for
`ctz(x | 1)`.

The fold lives in `bit_idioms_count_observe::exact` and reads its facts from
`bit_idioms_facts::Context::known`, which now crosses integer conversions
(`__builtin_ffs(x | 1)` scans `(int)(x | 1)`) and learns the bits a guard on
a shifted operand fixes. The Rust unit tests prove those rules over every
fact about a byte; this module pins what a program sees:

1. the folded functions contain no scan instruction at `-O2`, `-O3` and
   `-Os` (the levels that run `bit_idioms`), for x86-64 and i686;
2. a scan whose answer is not fixed (`ctz(x | 2)` is 0 or 1) is still a scan,
   so the first check cannot pass by the functions having been emptied;
3. every function returns what `-O0` returns, and what GCC and Clang return,
   over inputs that reach each bit position.
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

# name -> True when the scan must be gone.
FUNCTIONS = {
    "ctz_or1": True,
    "ctzll_or1": True,
    "ctz_signed_or1": True,
    "clz_msb": True,
    "clzll_msb": True,
    "ctz_shl_or": True,
    "ctzll_widened": True,
    "clzll_of_byte": True,
    "ffs_or1": True,
    "ctz_masked_guard": True,
    "clz_shift_guard": True,
    "clz_signed_shift_guard": True,
    "ctz_left_shift_guard": True,
    # The answer is not fixed: these keep their scan.
    "ctz_or2": False,
    "clz_or1": False,
    "ctz_plain": False,
}

SOURCE = r"""
int ctz_or1(unsigned x) { return __builtin_ctz(x | 1); }
int ctzll_or1(unsigned long long x) { return __builtin_ctzll(x | 1); }
int ctz_signed_or1(int x) { return __builtin_ctz(x | 1); }
int clz_msb(unsigned x) { return __builtin_clz(x | 0x80000000u); }
int clzll_msb(unsigned long long x) { return __builtin_clzll(x | 0x8000000000000000ull); }
int ctz_shl_or(unsigned x) { return __builtin_ctz(x << 3 | 8); }
int ctzll_widened(unsigned x) { return __builtin_ctzll((unsigned long long)(x << 2 | 4)); }
int clzll_of_byte(unsigned char x) { return __builtin_clzll((unsigned long long)(unsigned char)(x | 0x80)); }
int ffs_or1(unsigned x) { return __builtin_ffs((int)(x | 1)); }
int ctz_masked_guard(unsigned x) { if ((x & 7) == 4) return __builtin_ctz(x); return -1; }
int clz_shift_guard(unsigned x) { if (x >> 28 == 1) return __builtin_clz(x); return -1; }
int clz_signed_shift_guard(int x) { if (x >> 28 == 1) return __builtin_clz(x); return -1; }
int ctz_left_shift_guard(unsigned x) { if (x << 28 == 0x40000000u) return __builtin_ctz(x); return -1; }
int ctz_or2(unsigned x) { return __builtin_ctz(x | 2); }
int clz_or1(unsigned x) { return __builtin_clz(x | 1); }
int ctz_plain(unsigned x) { return x ? __builtin_ctz(x) : 32; }
"""

DRIVER = r"""
#include <stdio.h>
int main(void) {
    unsigned long long h = 1469598103934665603ull;
    unsigned x = 0;
    for (int i = 0; i < 4096; i++) {
        /* Every single bit, every pair with bit 0, and a walk of the rest. */
        if (i < 32) x = 1u << i;
        else if (i < 64) x = (1u << (i - 32)) | 1u;
        else if (i < 96) x = ~(1u << (i - 64));
        else x = x * 2654435761u + 0x9e3779b9u;
        unsigned long long w = ((unsigned long long)x << 32) | (x * 40503u);
        int r[] = {
            ctz_or1(x), ctzll_or1(w), ctz_signed_or1((int)x), clz_msb(x), clzll_msb(w),
            ctz_shl_or(x), ctzll_widened(x), clzll_of_byte((unsigned char)x), ffs_or1(x),
            ctz_masked_guard(x), clz_shift_guard(x), clz_signed_shift_guard((int)x),
            ctz_left_shift_guard(x), ctz_or2(x), clz_or1(x), ctz_plain(x),
        };
        for (unsigned k = 0; k < sizeof r / sizeof r[0]; k++)
            h = (h ^ (unsigned long long)(unsigned)r[k]) * 1099511628211ull;
        if (i < 6 || i == 34 || i == 66)
            printf("%08x: %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d %d\n", x, r[0], r[1],
                   r[2], r[3], r[4], r[5], r[6], r[7], r[8], r[9], r[10], r[11], r[12], r[13],
                   r[14], r[15]);
    }
    printf("%016llx\n", h);
    return 0;
}
"""

SCAN = re.compile(r"^\s*(?:rep\s+)?(?:bsf|bsr|tzcnt|lzcnt)[lqw]?\b", re.M)


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
class ExactCountFold(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix="exact-count-")
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

    def test_a_fixed_count_leaves_no_scan(self):
        for mode in ((), ("-m32",)):
            for level in ("-O2", "-O3", "-Os"):
                bodies = function_bodies(self.assembly(level, *mode))
                self.assertEqual(sorted(bodies), sorted(FUNCTIONS), (level, mode))
                for name, folded in FUNCTIONS.items():
                    with self.subTest(function=name, level=level, mode=mode):
                        scans = SCAN.findall(bodies[name])
                        if folded:
                            self.assertEqual(scans, [], bodies[name])
                        else:
                            self.assertNotEqual(scans, [], bodies[name])

    def test_the_folded_functions_return_what_the_scans_return(self):
        reference = self.run_program(str(LCCC), "-O0")
        self.assertEqual(len(reference.splitlines()), 9)
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
