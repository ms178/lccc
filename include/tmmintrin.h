/* CCC compiler bundled tmmintrin.h - SSSE3 intrinsics */
#ifndef _TMMINTRIN_H_INCLUDED
#define _TMMINTRIN_H_INCLUDED

#include <pmmintrin.h>

/* _mm_abs_epi16: absolute value of signed 16-bit integers (PABSW) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_abs_epi16(__m128i __a)
{
    return ((__m128i)(__builtin_ia32_pabsw128(__a)));
}

/* _mm_abs_epi8: absolute value of signed 8-bit integers (PABSB) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_abs_epi8(__m128i __a)
{
    return ((__m128i)(__builtin_ia32_pabsb128(__a)));
}

/* _mm_abs_epi32: absolute value of signed 32-bit integers (PABSD) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_abs_epi32(__m128i __a)
{
    return ((__m128i)(__builtin_ia32_pabsd128(__a)));
}

/* _mm_maddubs_epi16: multiply unsigned 8-bit * signed 8-bit, horizontally add
 * adjacent pairs to produce 8 x 16-bit results with saturation (PMADDUBSW).
 * __a is treated as unsigned, __b as signed. */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_maddubs_epi16(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_pmaddubsw128(__a, __b)));
}

/* _mm_shuffle_epi8: shuffle bytes according to control mask (PSHUFB) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_shuffle_epi8(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_pshufb128(__a, __b)));
}

/* _mm_alignr_epi8: concatenate __a (high) and __b (low) into a 32-byte
 * intermediate, shift right by __n bytes, return the low 16 bytes (PALIGNR) */
#define _mm_alignr_epi8(__a, __b, __n) \
    ((__m128i)(__builtin_ia32_palignr128((__a), (__b), ((__n) * 8))))


/* _mm_hadd_epi16: horizontal add adjacent pairs of 16-bit integers (PHADDW) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_hadd_epi16(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_phaddw128(__a, __b)));
}

/* _mm_hadd_epi32: horizontal add adjacent pairs of 32-bit integers (PHADDD) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_hadd_epi32(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_phaddd128(__a, __b)));
}

/* _mm_hsub_epi16: horizontal subtract adjacent pairs of 16-bit integers (PHSUBW) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_hsub_epi16(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_phsubw128(__a, __b)));
}

/* _mm_hsub_epi32: horizontal subtract adjacent pairs of 32-bit integers (PHSUBD) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_hsub_epi32(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_phsubd128(__a, __b)));
}

/* _mm_sign_epi8: negate/zero/keep bytes based on sign of __b (PSIGNB) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_sign_epi8(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_psignb128(__a, __b)));
}

/* _mm_sign_epi16: negate/zero/keep 16-bit ints based on sign (PSIGNW) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_sign_epi16(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_psignw128(__a, __b)));
}

/* _mm_sign_epi32: negate/zero/keep 32-bit ints based on sign (PSIGND) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_sign_epi32(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_psignd128(__a, __b)));
}

/* _mm_mulhrs_epi16: multiply high with round and scale (PMULHRSW) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_mulhrs_epi16(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_pmulhrsw128(__a, __b)));
}

/* _mm_hadds_epi16: horizontal add with saturation (PHADDSW) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_hadds_epi16(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_phaddsw128(__a, __b)));
}

/* _mm_hsubs_epi16: horizontal subtract with saturation (PHSUBSW) */
static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
_mm_hsubs_epi16(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_phsubsw128(__a, __b)));
}

#endif /* _TMMINTRIN_H_INCLUDED */
