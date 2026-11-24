"""Hold scripts/x86_intrinsic_census.py's own logic to account, without a compiler.

The census is a gate: these are the parts of it a green run never
exercises -- how a definition of GCC's headers is read, what the caller
written for it passes where the header wants an immediate, and that the
comparison with the record fails on every difference, a gained intrinsic
as much as a lost one.
"""
from __future__ import annotations

import sys
import tempfile
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

extern __inline __m128d __attribute__((__gnu_inline__, __always_inline__, __artificial__))
_mm_shuffle_pd(__m128d __A, __m128d __B, const int __mask)
{
  return (__m128d)__builtin_ia32_shufpd ((__v2df)__A, (__v2df)__B, __mask);
}
#define _mm_shuffle_pd(A, B, N) ((__m128d)__builtin_ia32_shufpd ((A), (B), (int)(N)))

extern __inline __m256i __attribute__((__gnu_inline__, __always_inline__, __artificial__))
_mm256_set_epi8  (char __q31, char __q30,
		  char __q01, char __q00)
{
  return __extension__ (__m256i)(__v32qi){ __q00, __q01, __q30, __q31 };
}

extern __inline
unsigned char __attribute__((__gnu_inline__, __always_inline__, __artificial__))
_mm_aesdecwide128kl_u8(__m128i __A[8], const __m128i __B[8], const void * __P)
{
  return __builtin_ia32_aesdecwide128kl_u8 ((__v2di *) __A, (__v2di *) __B, __P);
}

extern __inline __m128i __attribute__ ((__gnu_inline__, __always_inline__, __artificial__, __target__ ("xop")))
_mm_comeq_epi16(__m128i __A, __m128i __B)
{
  return (__m128i) __builtin_ia32_vpcomeqw ((__v8hi)__A, (__v8hi)__B);
}

extern __inline __m128 __attribute__((__gnu_inline__, __always_inline__, __artificial__))
_mm_msub_ps (__m128 __A, __m128 __B, __m128 __C)

{
  return (__m128) __builtin_ia32_vfmaddps ((__v4sf)__A, (__v4sf)__B, -(__v4sf)__C);
}

extern __inline__ __m256bh
__attribute__ ((__gnu_inline__, __always_inline__, __artificial__))
_mm256_add_pbh (__m256bh __A, __m256bh __B)
{
  return (__m256bh) __builtin_ia32_addbf16256 (__A, __B);
}

  extern __inline __m512h
  __attribute__ ((__gnu_inline__, __always_inline__, __artificial__))
  _mm512_cvtepu16_ph (__m512i __A)
  {
    return __builtin_ia32_vcvtuw2ph512_mask_round ((__v32hi) __A,
						   _mm512_setzero_ph (),
						   (__mmask32) -1, 4);
  }

extern __inline __m512h
__attribute__ ((__gnu_inline__, __always_inline__, __artificial__))
  _mm512_fmadd_ph (__m512h __A, __m512h __B, __m512h __C)
{
  return (__m512h) __builtin_ia32_vfmaddph512_mask ((__v32hf) __A, (__v32hf) __B, (__v32hf) __C,
						    (__mmask32) -1, 4);
}

static __inline__ void *
_mm_malloc (__SIZE_TYPE__ __size, __SIZE_TYPE__ __align)
{
  return 0;
}

extern int _mm_only_declared (int __A);
int
_not_inline (int __A)
{
  return __A;
}
"""

# What `gcc -aux-info` writes: the place, `N` for a prototype, `F` for a
# definition or `C` for a declaration, the declaration, the old-style form.
AUX_INFO = """/* compiled from: . */
/* /opt/gcc/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include/ia32intrin.h:31:NF */ extern int __bsfd (int __X); /* (__X) int __X; */
/* /opt/gcc/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include/xmmintrin.h:206:NF */ extern __m128 _mm_add_ps (__m128 __A, __m128 __B); /* (__A, __B) __m128 __A; __m128 __B; */
/* /opt/gcc/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include/xmmintrin.h:1300:NF */ extern void _mm_pause (void); /* () */
/* /opt/gcc/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include/mm_malloc.h:30:NF */ static void *_mm_malloc (size_t __size, size_t __align); /* (__size, __align) size_t __size; size_t __align; */
/* /usr/include/stdlib.h:672:NC */ extern void *malloc (size_t); /* (size_t) */
/* /opt/gcc/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include/xmmintrin.h:99:NC */ extern __m128 _mm_declared (__m128); /* (__m128) */
/* /opt/gcc/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include/ssp/string.h:10:NF */ extern int _in_a_subdirectory (int __X); /* (__X) int __X; */
/* c.c:3:NF */ extern int _the_program (int a); /* (a) int a; */
"""


class Reading(unittest.TestCase):
    def setUp(self):
        self.functions = census.functions_of(HEADER)

    def test_every_definition_is_found_once(self):
        self.assertEqual(sorted(self.functions), [
            "_addcarry_u64", "_mm256_add_pbh", "_mm256_set_epi8", "_mm512_add_round_pd",
            "_mm512_cvtepu16_ph", "_mm512_fmadd_ph", "_mm512_mask_add_epi32",
            "_mm_aesdecwide128kl_u8", "_mm_clflushopt", "_mm_comeq_epi16", "_mm_malloc",
            "_mm_msub_ps", "_mm_set1_epi32", "_mm_shuffle_epi32", "_mm_shuffle_pd",
            "_rdpid_u32"])

    def test_a_blank_line_may_stand_in_front_of_the_body(self):
        returns, parameters, _ = self.functions["_mm_msub_ps"]
        self.assertEqual(returns, "__m128")
        self.assertEqual(parameters, ["__m128 __A", "__m128 __B", "__m128 __C"])

    def test_inline_has_two_spellings(self):
        # 292 of GCC 16.2's definitions say `__inline__`: all of AVX10.2.
        self.assertEqual(self.functions["_mm256_add_pbh"][0], "__m256bh")
        self.assertEqual(self.functions["_mm_malloc"][:2],
                         ("void *", ["__SIZE_TYPE__ __size", "__SIZE_TYPE__ __align"]))

    def test_a_definition_may_be_indented_whole_or_at_its_name(self):
        self.assertEqual(self.functions["_mm512_cvtepu16_ph"][:2], ("__m512h", ["__m512i __A"]))
        self.assertEqual(self.functions["_mm512_fmadd_ph"][:2],
                         ("__m512h", ["__m512h __A", "__m512h __B", "__m512h __C"]))

    def test_the_spacing_in_front_of_the_parameters_does_not_matter(self):
        # `_mm_shuffle_pd(`, `_mm256_set_epi8  (`: read as `_mm_add_ps (` is.
        # A pattern that wanted one space left out 148 of GCC 16.2's 7,174
        # definitions, every XOP intrinsic among them.
        returns, parameters, twin = self.functions["_mm_shuffle_pd"]
        self.assertEqual((returns, twin), ("__m128d", True))
        self.assertEqual(parameters, ["__m128d __A", "__m128d __B", "const int __mask"])
        returns, parameters, _ = self.functions["_mm256_set_epi8"]
        self.assertEqual(returns, "__m256i")
        self.assertEqual(parameters, ["char __q31", "char __q30", "char __q01", "char __q00"])

    def test_the_return_type_may_stand_on_a_line_of_its_own(self):
        returns, parameters, _ = self.functions["_mm_aesdecwide128kl_u8"]
        self.assertEqual(returns, "unsigned char")
        self.assertEqual(parameters, ["__m128i __A[8]", "const __m128i __B[8]",
                                      "const void * __P"])

    def test_an_attribute_with_parentheses_of_its_own_is_no_part_of_the_type(self):
        self.assertEqual(self.functions["_mm_comeq_epi16"][0], "__m128i")
        self.assertEqual(
            census.without_attributes(' __m128i __attribute__ ((__a__, __target__ ("xop"))) x'),
            " __m128i  x")
        self.assertEqual(census.without_attributes("__attribute__((a)) int __attribute__((b))"),
                         " int ")

    def test_a_declaration_and_a_function_that_is_not_inline_are_no_definitions(self):
        self.assertNotIn("_mm_only_declared", self.functions)
        self.assertNotIn("_not_inline", self.functions)

    def test_the_definitions_gcc_lists_are_read_from_its_own_account(self):
        include = Path("/opt/gcc/lib/gcc/x86_64-pc-linux-gnu/16.2.0/include")
        self.assertEqual(census.aux_definitions_of(AUX_INFO, include), {
            "ia32intrin.h": {"__bsfd"},
            "xmmintrin.h": {"_mm_add_ps", "_mm_pause"},
            "mm_malloc.h": {"_mm_malloc"},
        })
        # Another compiler's directory holds none of them.
        self.assertEqual(census.aux_definitions_of(AUX_INFO, Path("/usr/include")), {})

    def test_the_headers_are_the_intrin_headers_and_the_two_named_otherwise(self):
        with tempfile.TemporaryDirectory() as directory:
            include = Path(directory)
            for name in ("xmmintrin.h", "mm3dnow.h", "mm_malloc.h", "cpuid.h", "float.h",
                         "avx512fintrin.h"):
                (include / name).write_text("")
            self.assertEqual([header.name for header in census.headers_of(include)],
                             ["avx512fintrin.h", "mm3dnow.h", "mm_malloc.h", "xmmintrin.h"])

    def test_an_array_parameter_is_the_pointer_it_decays_to(self):
        self.assertEqual(census.parameter_of("__m128i __A[8]"), ("__m128i *", "__A"))
        self.assertEqual(census.parameter_of("const __m128i __B[8]"), ("const __m128i *", "__B"))
        self.assertEqual(census.parameter_of("const void * __P"), ("const void *", "__P"))
        self.assertEqual(census.parameter_of("unsigned long long *__P"),
                         ("unsigned long long *", "__P"))
        self.assertEqual(
            census.caller(3, "_mm_aesdecwide128kl_u8",
                          *self.functions["_mm_aesdecwide128kl_u8"]),
            "unsigned char t3(__m128i * a0, const __m128i * a1, const void * a2) "
            "{ return _mm_aesdecwide128kl_u8(a0, a1, a2); }")

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
