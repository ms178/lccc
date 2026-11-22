"""Hold scripts/x86_intrinsic_census.py's own logic to account, without a compiler.

The census is a gate: these are the parts of it a green run never
exercises -- how a definition of GCC's headers is read, what the caller
written for it passes where the header wants an immediate, and that the
comparison with the record fails on every difference, a gained intrinsic
as much as a lost one.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))

from scripts import x86_intrinsic_census as census  # noqa: E402

# Definitions as GCC 16.2's headers lay them out: attributes on a line of
# their own (avx512fintrin.h), behind the return type (adxintrin.h), a
# `void` parameter list, an immediate with a function-like macro twin, and
# what is no intrinsic's definition.
HEADER = """\
extern __inline __m512i
__attribute__ ((__gnu_inline__, __always_inline__, __artificial__))
_mm512_mask_add_epi32 (__m512i __W, __mmask16 __U, __m512i __A, __m512i __B)
{
  return (__m512i) __builtin_ia32_paddd512_mask (__A, __B, __W, __U);
}

extern __inline unsigned char
__attribute__((__gnu_inline__, __always_inline__, __artificial__))
_addcarry_u64 (unsigned char __CF, unsigned long long __X,
	       unsigned long long __Y, unsigned long long *__P)
{
  return __builtin_ia32_addcarryx_u64 (__CF, __X, __Y, __P);
}

extern __inline void __attribute__((__gnu_inline__, __always_inline__, __artificial__))
_mm_clflushopt (void *__A)
{
  __builtin_ia32_clflushopt (__A);
}

extern __inline unsigned int
__attribute__((__gnu_inline__, __always_inline__, __artificial__))
_rdpid_u32 (void)
{
  return __builtin_ia32_rdpid ();
}

#ifdef __OPTIMIZE__
extern __inline __m512d
__attribute__ ((__gnu_inline__, __always_inline__, __artificial__))
_mm512_add_round_pd (__m512d __A, __m512d __B, const int __R)
{
  return (__m512d) __builtin_ia32_addpd512_mask (__A, __B, __W, -1, __R);
}

extern __inline __m128i
__attribute__ ((__gnu_inline__, __always_inline__, __artificial__))
_mm_shuffle_epi32 (__m128i __A, const int __mask)
{
  return (__m128i)__builtin_ia32_pshufd ((__v4si)__A, __mask);
}
#else
#define _mm512_add_round_pd(A, B, C) \\
    (__m512d)__builtin_ia32_addpd512_mask(A, B, (__v8df)_mm512_undefined_pd(), -1, C)
#define _mm_shuffle_epi32(A, N) \\
  ((__m128i)__builtin_ia32_pshufd ((__v4si)(__m128i)(A), (int)(N)))
#endif

extern __inline __m128i __attribute__((__gnu_inline__, __always_inline__, __artificial__))
_mm_set1_epi32 (int __A)
{
  return _mm_set_epi32 (__A, __A, __A, __A);
}

static void helper (int x)
{
}
#define _MM_SHUFFLE(z, y, x, w) (((z) << 6) | ((y) << 4) | ((x) << 2) | (w))
"""


class Reading(unittest.TestCase):
    def setUp(self):
        self.functions = census.functions_of(HEADER)

    def test_every_definition_is_found_once(self):
        self.assertEqual(sorted(self.functions), [
            "_addcarry_u64", "_mm512_add_round_pd", "_mm512_mask_add_epi32",
            "_mm_clflushopt", "_mm_set1_epi32", "_mm_shuffle_epi32", "_rdpid_u32"])

    def test_types_are_read(self):
        returns, parameters, twin = self.functions["_mm512_mask_add_epi32"]
        self.assertEqual(returns, "__m512i")
        self.assertEqual(parameters, ["__m512i __W", "__mmask16 __U", "__m512i __A",
                                      "__m512i __B"])
        self.assertFalse(twin)
        # Attributes behind the return type are no part of it, and a
        # parameter list may run over lines.
        returns, parameters, _ = self.functions["_addcarry_u64"]
        self.assertEqual(returns, "unsigned char")
        self.assertEqual(len(parameters), 4)
        self.assertEqual(parameters[3], "unsigned long long *__P")
        self.assertEqual(self.functions["_mm_clflushopt"][0], "void")
        self.assertEqual(self.functions["_rdpid_u32"][1], [])

    def test_a_macro_twin_marks_the_immediates(self):
        self.assertTrue(self.functions["_mm512_add_round_pd"][2])
        self.assertTrue(self.functions["_mm_shuffle_epi32"][2])
        # `int` without a twin is a value like another.
        self.assertFalse(self.functions["_mm_set1_epi32"][2])


class Callers(unittest.TestCase):
    def setUp(self):
        self.functions = census.functions_of(HEADER)

    def line(self, name: str) -> str:
        return census.caller(0, name, *self.functions[name])

    def test_arguments_have_the_parameters_types(self):
        self.assertEqual(
            self.line("_mm512_mask_add_epi32"),
            "__m512i t0(__m512i a0, __mmask16 a1, __m512i a2, __m512i a3) "
            "{ return _mm512_mask_add_epi32(a0, a1, a2, a3); }")
        self.assertEqual(
            self.line("_addcarry_u64"),
            "unsigned char t0(unsigned char a0, unsigned long long a1, unsigned long long a2, "
            "unsigned long long * a3) { return _addcarry_u64(a0, a1, a2, a3); }")

    def test_void_in_either_place(self):
        self.assertEqual(self.line("_mm_clflushopt"),
                         "void t0(void * a0) { _mm_clflushopt(a0); }")
        self.assertEqual(self.line("_rdpid_u32"),
                         "unsigned int t0(void) { return _rdpid_u32(); }")

    def test_an_immediate_is_a_constant(self):
        # A rounding mode has to be one GCC accepts: 8 is
        # _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC.
        self.assertEqual(self.line("_mm512_add_round_pd"),
                         "__m512d t0(__m512d a0, __m512d a1) "
                         "{ return _mm512_add_round_pd(a0, a1, 8); }")
        self.assertEqual(self.line("_mm_shuffle_epi32"),
                         "__m128i t0(__m128i a0) { return _mm_shuffle_epi32(a0, 1); }")
        self.assertEqual(self.line("_mm_set1_epi32"),
                         "__m128i t0(int a0) { return _mm_set1_epi32(a0); }")

    def test_callers_are_named_by_index(self):
        functions = census.functions_of(HEADER)
        names = sorted(functions)
        lines = [census.caller(i, name, *functions[name]) for i, name in enumerate(names)]
        self.assertEqual(len({line.split("(")[0].split()[-1] for line in lines}), len(names))


class Record(unittest.TestCase):
    REPORT = {
        "adxintrin.h": (6, ["_addcarry_u32", "_addcarry_u64"], []),
        "avxintrin.h": (180, [], ["_mm256_odd"]),
    }

    def test_the_text_of_a_census(self):
        text = census.expectation_text(self.REPORT, "16.2.0")
        self.assertIn("gcc 16.2.0\ncallable 186\nmissing 2\n", text)
        self.assertIn(f"header adxintrin.h 6 2 {census.digest_of(['_addcarry_u64', '_addcarry_u32'])}\n",
                      text)
        self.assertIn(f"header avxintrin.h 180 0 {census.digest_of([])}\n", text)

    def test_the_digest_is_of_the_names_not_their_order(self):
        self.assertEqual(census.digest_of(["b", "a"]), census.digest_of(["a", "b"]))
        self.assertNotEqual(census.digest_of(["a"]), census.digest_of(["a", "b"]))

    def test_any_difference_fails_in_either_direction(self):
        recorded = census.expectation_text(self.REPORT, "16.2.0")
        self.assertEqual(census.failures_against(recorded, recorded), [])
        # An intrinsic lost.
        worse = dict(self.REPORT, **{"avxintrin.h": (180, ["_mm256_add_ps"], [])})
        problems = census.failures_against(census.expectation_text(worse, "16.2.0"), recorded)
        self.assertTrue(any("found, not expected:  missing 3" in p for p in problems), problems)
        self.assertTrue(any("header avxintrin.h 180 1" in p for p in problems), problems)
        # An intrinsic gained: the record has to say so.
        better = dict(self.REPORT, **{"adxintrin.h": (6, ["_addcarry_u32"], [])})
        problems = census.failures_against(census.expectation_text(better, "16.2.0"), recorded)
        self.assertTrue(any("expected, not found:  missing 2" in p for p in problems), problems)
        # Another name missing in place of one: the counts agree, the digest does not.
        other = dict(self.REPORT, **{"adxintrin.h": (6, ["_addcarry_u32", "_subborrow_u64"], [])})
        problems = census.failures_against(census.expectation_text(other, "16.2.0"), recorded)
        self.assertEqual(len(problems), 2, problems)
        # Another GCC.
        problems = census.failures_against(census.expectation_text(self.REPORT, "16.3.0"), recorded)
        self.assertIn("found, not expected:  gcc 16.3.0", problems)

    def test_comments_are_no_part_of_the_record(self):
        recorded = census.expectation_text(self.REPORT, "16.2.0")
        self.assertEqual(census.failures_against("# another note\n" + recorded, recorded), [])


class CommandLine(unittest.TestCase):
    def test_a_missing_compiler_is_reported(self):
        self.assertEqual(census.main(["--lccc", "/nonexistent/lccc"]), 2)

    def test_the_committed_record_is_well_formed(self):
        lines = [line for line in census.EXPECTED.read_text().splitlines()
                 if line and not line.startswith("#")]
        self.assertEqual(lines[0], f"gcc {census.GCC_VERSION}")
        callable_, missing = int(lines[1].split()[1]), int(lines[2].split()[1])
        headers = [line.split() for line in lines[3:]]
        self.assertTrue(all(row[0] == "header" and len(row) == 5 for row in headers))
        self.assertEqual(sum(int(row[2]) for row in headers), callable_)
        self.assertEqual(sum(int(row[3]) for row in headers), missing)
        self.assertTrue(all(int(row[3]) <= int(row[2]) for row in headers))
        self.assertEqual([row[1] for row in headers], sorted(row[1] for row in headers))


if __name__ == "__main__":
    unittest.main()
