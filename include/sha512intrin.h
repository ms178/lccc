/* CCC compiler bundled sha512intrin.h - SHA-512 intrinsics (VEX.256)
 *
 * Each wrapper lowers to the one VEX.256 SHA-512 instruction it names, through
 * the matching `__builtin_ia32_vsha512*` builtin.  Nothing is computed in
 * software.
 *
 * Each wrapper carries `target("sha512,avx")`, so a call from a function
 * without `-msha512` is rejected at the public name, as GCC 16.2 does.  GCC
 * makes `-msha512` imply AVX (VEX.256 needs AVX) but not AVX2, and so does this
 * header.
 *
 * Operand order follows Intel's definitions:
 *   _mm256_sha512msg1_epi64(a, b)       dst = a (W[0..3]), b = xmm (W[4] in low qword)
 *   _mm256_sha512msg2_epi64(a, b)       dst = a (partial), b = ymm (W[14], W[15] in qwords 2, 3)
 *   _mm256_sha512rnds2_epi64(a, b, k)   dst = a (CDGH), b = ABEF, k = xmm W+K (low two qwords).
 *                                       The result is the updated ABEF.
 */
#ifndef _SHA512INTRIN_H_INCLUDED
#define _SHA512INTRIN_H_INCLUDED

#include <avxintrin.h>

/* The builtins are declared on V4DI (256-bit) and V2DI (128-bit) vectors.
 * The wrappers reinterpret the operands to those shapes, a bit-for-bit no-op. */
typedef long long __ccc_sha512_v4di __attribute__((__vector_size__(32)));
typedef long long __ccc_sha512_v2di __attribute__((__vector_size__(16)));

static __inline__ __m256i __attribute__((__always_inline__, target("sha512,avx")))
_mm256_sha512msg1_epi64(__m256i __A, __m128i __B)
{
    return ((__m256i)(__builtin_ia32_vsha512msg1((__ccc_sha512_v4di)__A, (__ccc_sha512_v2di)__B)));
}

static __inline__ __m256i __attribute__((__always_inline__, target("sha512,avx")))
_mm256_sha512msg2_epi64(__m256i __A, __m256i __B)
{
    return ((__m256i)(__builtin_ia32_vsha512msg2((__ccc_sha512_v4di)__A, (__ccc_sha512_v4di)__B)));
}

static __inline__ __m256i __attribute__((__always_inline__, target("sha512,avx")))
_mm256_sha512rnds2_epi64(__m256i __A, __m256i __B, __m128i __C)
{
    return ((__m256i)(__builtin_ia32_vsha512rnds2((__ccc_sha512_v4di)__A, (__ccc_sha512_v4di)__B,
                                                  (__ccc_sha512_v2di)__C)));
}

#endif /* _SHA512INTRIN_H_INCLUDED */
