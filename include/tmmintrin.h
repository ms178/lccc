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
    ((__m128i)(__builtin_ia32_palignr128((__a), (__b), ((int)(__n) * 8))))


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

/* --- SSSE3 forms on MMX registers (_mm_*_pi8/_pi16/_pi32, _mm_alignr_pi8).
 * GCC 16.2 declares these in tmmintrin.h. Each one runs the 128-bit form on the
 * operands' low quadwords, so the bit pattern matches the MMX instruction.
 * Element-wise forms zero-extend the operands and keep the low 64 bits. The
 * horizontal and byte-index forms first place the two quadwords side by side
 * in one XMM register, so that lane order matches the MMX definition:
 * a in the low 64 bits (lanes of a before b), then b. --- */

static __inline__ __m128i __attribute__((__always_inline__, target("ssse3")))
__lccc_mm64_pair(__m64 __lo, __m64 __hi)
{
    return _mm_unpacklo_epi64(_mm_movpi64_epi64(__lo), _mm_movpi64_epi64(__hi));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_abs_pi8(__m64 __a)
{
    return _mm_movepi64_pi64(_mm_abs_epi8(_mm_movpi64_epi64(__a)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_abs_pi16(__m64 __a)
{
    return _mm_movepi64_pi64(_mm_abs_epi16(_mm_movpi64_epi64(__a)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_abs_pi32(__m64 __a)
{
    return _mm_movepi64_pi64(_mm_abs_epi32(_mm_movpi64_epi64(__a)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_sign_pi8(__m64 __a, __m64 __b)
{
    return _mm_movepi64_pi64(_mm_sign_epi8(_mm_movpi64_epi64(__a), _mm_movpi64_epi64(__b)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_sign_pi16(__m64 __a, __m64 __b)
{
    return _mm_movepi64_pi64(_mm_sign_epi16(_mm_movpi64_epi64(__a), _mm_movpi64_epi64(__b)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_sign_pi32(__m64 __a, __m64 __b)
{
    return _mm_movepi64_pi64(_mm_sign_epi32(_mm_movpi64_epi64(__a), _mm_movpi64_epi64(__b)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_mulhrs_pi16(__m64 __a, __m64 __b)
{
    return _mm_movepi64_pi64(_mm_mulhrs_epi16(_mm_movpi64_epi64(__a), _mm_movpi64_epi64(__b)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_maddubs_pi16(__m64 __a, __m64 __b)
{
    return _mm_movepi64_pi64(_mm_maddubs_epi16(_mm_movpi64_epi64(__a), _mm_movpi64_epi64(__b)));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_hadd_pi16(__m64 __a, __m64 __b)
{
    __m128i __x = __lccc_mm64_pair(__a, __b);
    return _mm_movepi64_pi64(_mm_hadd_epi16(__x, __x));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_hadd_pi32(__m64 __a, __m64 __b)
{
    __m128i __x = __lccc_mm64_pair(__a, __b);
    return _mm_movepi64_pi64(_mm_hadd_epi32(__x, __x));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_hadds_pi16(__m64 __a, __m64 __b)
{
    __m128i __x = __lccc_mm64_pair(__a, __b);
    return _mm_movepi64_pi64(_mm_hadds_epi16(__x, __x));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_hsub_pi16(__m64 __a, __m64 __b)
{
    __m128i __x = __lccc_mm64_pair(__a, __b);
    return _mm_movepi64_pi64(_mm_hsub_epi16(__x, __x));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_hsub_pi32(__m64 __a, __m64 __b)
{
    __m128i __x = __lccc_mm64_pair(__a, __b);
    return _mm_movepi64_pi64(_mm_hsub_epi32(__x, __x));
}

static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_hsubs_pi16(__m64 __a, __m64 __b)
{
    __m128i __x = __lccc_mm64_pair(__a, __b);
    return _mm_movepi64_pi64(_mm_hsubs_epi16(__x, __x));
}

/* PSHUFB on MMX: a control byte with bit 7 set yields zero, otherwise bits 2:0
 * select a byte of __a. Masking with 0x87 keeps exactly those bits, so the
 * 128-bit form (which reads bits 3:0) selects the same byte. */
static __inline__ __m64 __attribute__((__always_inline__, target("ssse3")))
_mm_shuffle_pi8(__m64 __a, __m64 __b)
{
    __m128i __ctl = _mm_and_si128(_mm_movpi64_epi64(__b), _mm_set1_epi8((char)0x87));
    return _mm_movepi64_pi64(_mm_shuffle_epi8(_mm_movpi64_epi64(__a), __ctl));
}

/* PALIGNR on MMX: bytes [n, n+8) of the 16-byte value (__a:__b), __b low, __a
 * high; n >= 16 gives zero. The high operand of the 128-bit form is zero, so
 * bytes past the 16-byte value read zero, as the MMX definition requires. */
#define _mm_alignr_pi8(__a, __b, __n)                                            \
    _mm_movepi64_pi64(_mm_alignr_epi8(_mm_setzero_si128(),                       \
                                      __lccc_mm64_pair((__b), (__a)), (__n)))

#endif /* _TMMINTRIN_H_INCLUDED */
