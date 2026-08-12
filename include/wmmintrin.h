/* CCC compiler bundled wmmintrin.h - AES-NI and CLMUL intrinsics */
#ifndef _WMMINTRIN_H_INCLUDED
#define _WMMINTRIN_H_INCLUDED

#include <emmintrin.h>

/* === AES-NI intrinsics === */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesenc_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesenc128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesenclast_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesenclast128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesdec_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesdec128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesdeclast_si128(__m128i __V, __m128i __R)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesdeclast128(__V, __R));
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_aesimc_si128(__m128i __V)
{
    return __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aesimc128(__V));
}

/* _mm_aeskeygenassist_si128 requires a compile-time constant imm8 */
#define _mm_aeskeygenassist_si128(V, I) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_aeskeygenassist128((V), (I)))

/* === CLMUL (carry-less multiplication) === */

/* _mm_clmulepi64_si128 requires a compile-time constant imm8 */
#define _mm_clmulepi64_si128(X, Y, I) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pclmulqdq128((X), (Y), (I)))

/* === 256-bit AES-NI / VPCLMULQDQ ===
 *
 * The GCC/Clang system headers gate _mm256_aes*_epi128 and
 * _mm256_clmulepi64_epi128 behind __VAES__ / (__VPCLMULQDQ__ && __AVX512F__),
 * which are never defined by a plain -mavx2 build. Without a declaration
 * there is no function signature, and the struct-copy-init lowering
 * misclassifies the call as a small packed return: it spilled the intrinsic's
 * result ADDRESS (8 bytes) and memcpy'd 32 bytes "through" it, corrupting
 * the destination (clmul256 regression). The builtin intercept handles the
 * bodies; these declarations exist so the signatures (sret for 32-byte
 * vector returns) are seeded from the header, exactly like the 128-bit
 * intrinsics above. */
#ifdef __AVX2__
#include <avxintrin.h>
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_clmulepi64_epi128(__m256i __A, __m256i __B, const int __C)
{
    return __CCC_M256I_FROM_BUILTIN(__builtin_ia32_vpclmulqdq256(__A, __B, __C));
}

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
#endif /* __AVX2__ */

#endif /* _WMMINTRIN_H_INCLUDED */
