/* CCC compiler bundled wmmintrin.h - AES-NI and CLMUL intrinsics */
#ifndef _WMMINTRIN_H_INCLUDED
#define _WMMINTRIN_H_INCLUDED

#include <emmintrin.h>

/* === AES-NI intrinsics === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesenc_si128(__m128i __V, __m128i __R)
{
    return ((__m128i)(__builtin_ia32_aesenc128(__V, __R)));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesenclast_si128(__m128i __V, __m128i __R)
{
    return ((__m128i)(__builtin_ia32_aesenclast128(__V, __R)));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesdec_si128(__m128i __V, __m128i __R)
{
    return ((__m128i)(__builtin_ia32_aesdec128(__V, __R)));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesdeclast_si128(__m128i __V, __m128i __R)
{
    return ((__m128i)(__builtin_ia32_aesdeclast128(__V, __R)));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesimc_si128(__m128i __V)
{
    return ((__m128i)(__builtin_ia32_aesimc128(__V)));
}

/* _mm_aeskeygenassist_si128 requires a compile-time constant imm8 */
#define _mm_aeskeygenassist_si128(V, I) \
    ((__m128i)(__builtin_ia32_aeskeygenassist128((V), (I))))

/* === CLMUL (carry-less multiplication) === */

/* _mm_clmulepi64_si128 requires a compile-time constant imm8 */
#define _mm_clmulepi64_si128(X, Y, I) \
    ((__m128i)(__builtin_ia32_pclmulqdq128((X), (Y), (I))))

/* === 256-bit AES-NI / VPCLMULQDQ ===
 *
 * Declared whatever the command line enables, as GCC's <immintrin.h>
 * declares them (vaesintrin.h, under a target pragma): whether a call is
 * allowed is the builtin's own question, asked per function, so
 * `-mvaes -mno-avx2`, a bare `-mvaes` and `__attribute__((target("vaes")))`
 * get the instruction and everything else gets the diagnostic. The
 * declaration is what gives the call its type. This section used to stand
 * under `#ifdef __AVX2__`; VAES at 256 bits needs AVX, not AVX2, and in
 * the configurations between the two the public name had no declaration:
 * the call was the implicit `int`, with the address of the result as its
 * value. */
#include <avxintrin.h>
/* A macro, as _mm_clmulepi64_si128 is: the selector is an immediate of the
 * instruction, and a function's parameter is not one. As a function this
 * compiled to `vpclmulqdq $0` for every selector at -O0. */
#define _mm256_clmulepi64_epi128(__A, __B, __C) \
    (__CCC_M256I_FROM_BUILTIN(__builtin_ia32_vpclmulqdq256((__A), (__B), (__C))))

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_aesenc_epi128(__m256i __V, __m256i __R)
{
    return __CCC_M256I_FROM_BUILTIN(__builtin_ia32_aesenc256(__V, __R));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_aesenclast_epi128(__m256i __V, __m256i __R)
{
    return __CCC_M256I_FROM_BUILTIN(__builtin_ia32_aesenclast256(__V, __R));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_aesdec_epi128(__m256i __V, __m256i __R)
{
    return __CCC_M256I_FROM_BUILTIN(__builtin_ia32_aesdec256(__V, __R));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_aesdeclast_epi128(__m256i __V, __m256i __R)
{
    return __CCC_M256I_FROM_BUILTIN(__builtin_ia32_aesdeclast256(__V, __R));
}

#endif /* _WMMINTRIN_H_INCLUDED */
