/* CCC compiler bundled shaintrin.h - SHA-NI intrinsics
 *
 * Every wrapper lowers to the one SHA-NI instruction it names, through the
 * matching `__builtin_ia32_sha*` builtin.  Nothing is computed in software:
 * the scalar bodies this header used to carry read `__m128i` through
 * `unsigned int *`, a strict-aliasing violation.
 *
 * Each wrapper carries `target("sha")`, so a call from a function without
 * `-msha` (or `target("sha")`) is rejected at the public name, as GCC does.
 *
 * Operand order follows Intel's SHA-NI definitions:
 *   _mm_sha1nexte_epu32(a, b)        dst = a, src = b
 *   _mm_sha1msg1_epu32(a, b)         dst = a, src = b
 *   _mm_sha1msg2_epu32(a, b)         dst = a, src = b
 *   _mm_sha1rnds4_epu32(a, b, imm)   dst = a, src = b, imm in [0, 3]
 *   _mm_sha256msg1_epu32(a, b)       dst = a, src = b
 *   _mm_sha256msg2_epu32(a, b)       dst = a, src = b
 *   _mm_sha256rnds2_epu32(a, b, k)   dst = a (CDGH), src = b (ABEF), k = WK
 *                                    (the instruction reads k from %xmm0)
 */
#ifndef _SHAINTRIN_H_INCLUDED
#define _SHAINTRIN_H_INCLUDED

#include <emmintrin.h>

/* The SHA-NI builtins are declared on 4 x 32-bit vectors (GCC's `V4SI`).
 * The wrappers below reinterpret the `__m128i` operands to that shape, which
 * is a bit-for-bit no-op. */
typedef int __ccc_sha_v4si __attribute__((__vector_size__(16)));

/* ========================================================================
 * SHA-1
 * ======================================================================== */

/* SHA1NEXTE: E' = ROL(a[3], 30) + b[3]; lanes 0..2 of the result are b's. */
static __inline__ __m128i __attribute__((__always_inline__, target("sha")))
_mm_sha1nexte_epu32(__m128i __A, __m128i __B)
{
    return ((__m128i)(__builtin_ia32_sha1nexte((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B)));
}

/* SHA1MSG1: first message-schedule step (W[i] ^ W[i+2] per lane). */
static __inline__ __m128i __attribute__((__always_inline__, target("sha")))
_mm_sha1msg1_epu32(__m128i __A, __m128i __B)
{
    return ((__m128i)(__builtin_ia32_sha1msg1((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B)));
}

/* SHA1MSG2: second message-schedule step (rotate-left-1 of the XOR chain). */
static __inline__ __m128i __attribute__((__always_inline__, target("sha")))
_mm_sha1msg2_epu32(__m128i __A, __m128i __B)
{
    return ((__m128i)(__builtin_ia32_sha1msg2((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B)));
}

/* SHA1RNDS4: four SHA-1 rounds.  The immediate selects the round function
 * and constant (0..3) and must be a compile-time constant, so this is a
 * macro, as in GCC's unoptimised header. */
static __inline__ __m128i __attribute__((__always_inline__, target("sha")))
_mm_sha1rnds4_epu32(__m128i __A, __m128i __B, const int __I)
{
    switch (__I & 3) {
    case 0: return (__m128i)__builtin_ia32_sha1rnds4((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B, 0);
    case 1: return (__m128i)__builtin_ia32_sha1rnds4((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B, 1);
    case 2: return (__m128i)__builtin_ia32_sha1rnds4((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B, 2);
    default: return (__m128i)__builtin_ia32_sha1rnds4((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B, 3);
    }
}

/* ========================================================================
 * SHA-256
 * ======================================================================== */

/* SHA256RNDS2: two SHA-256 rounds.  `__A` holds CDGH, `__B` holds ABEF,
 * `__C` holds the two pre-added message+round-constant words (WK). */
static __inline__ __m128i __attribute__((__always_inline__, target("sha")))
_mm_sha256rnds2_epu32(__m128i __A, __m128i __B, __m128i __C)
{
    return ((__m128i)(__builtin_ia32_sha256rnds2((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B, (__ccc_sha_v4si)__C)));
}

/* SHA256MSG1: first message-schedule step (sigma0 of the previous words). */
static __inline__ __m128i __attribute__((__always_inline__, target("sha")))
_mm_sha256msg1_epu32(__m128i __A, __m128i __B)
{
    return ((__m128i)(__builtin_ia32_sha256msg1((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B)));
}

/* SHA256MSG2: second message-schedule step (sigma1 of the new words). */
static __inline__ __m128i __attribute__((__always_inline__, target("sha")))
_mm_sha256msg2_epu32(__m128i __A, __m128i __B)
{
    return ((__m128i)(__builtin_ia32_sha256msg2((__ccc_sha_v4si)__A, (__ccc_sha_v4si)__B)));
}

#endif /* _SHAINTRIN_H_INCLUDED */
