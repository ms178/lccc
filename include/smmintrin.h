/* CCC compiler bundled smmintrin.h - SSE4.1 / SSE4.2 intrinsics */
#ifndef _SMMINTRIN_H_INCLUDED
#define _SMMINTRIN_H_INCLUDED

#include <tmmintrin.h>
/* SSE4.2 shipped POPCNT; GCC's smmintrin.h carries its intrinsics too. */
#include <popcntintrin.h>

/* Rounding-mode macros for _mm_round_* and _mm256_round_* (and therefore the
 * AVX1 _mm256_ceil/floor_{ps,pd}, which expand to _mm256_round_* with these
 * constants).  Values and composition match GCC 16.2 smmintrin.h exactly. */
#define _MM_FROUND_TO_NEAREST_INT 0x00
#define _MM_FROUND_TO_NEG_INF 0x01
#define _MM_FROUND_TO_POS_INF 0x02
#define _MM_FROUND_TO_ZERO 0x03
#define _MM_FROUND_CUR_DIRECTION 0x04

#define _MM_FROUND_RAISE_EXC 0x00
#define _MM_FROUND_NO_EXC 0x08

#define _MM_FROUND_NINT (_MM_FROUND_TO_NEAREST_INT | _MM_FROUND_RAISE_EXC)
#define _MM_FROUND_FLOOR (_MM_FROUND_TO_NEG_INF | _MM_FROUND_RAISE_EXC)
#define _MM_FROUND_CEIL (_MM_FROUND_TO_POS_INF | _MM_FROUND_RAISE_EXC)
#define _MM_FROUND_TRUNC (_MM_FROUND_TO_ZERO | _MM_FROUND_RAISE_EXC)
#define _MM_FROUND_RINT (_MM_FROUND_CUR_DIRECTION | _MM_FROUND_RAISE_EXC)
#define _MM_FROUND_NEARBYINT (_MM_FROUND_CUR_DIRECTION | _MM_FROUND_NO_EXC)

/* === SSE4.1 insert/extract intrinsics === */

/* _mm_extract_epi8: extract 8-bit int from lane (PEXTRB) */
#define _mm_extract_epi8(a, imm) \
    ((int)(unsigned char)__builtin_ia32_pextrb128((a), (imm)))

/* _mm_extract_epi32: extract 32-bit int from lane (PEXTRD) */
#define _mm_extract_epi32(a, imm) \
    ((int)__builtin_ia32_pextrd128((a), (imm)))

/* _mm_extract_epi64: extract 64-bit int from lane (PEXTRQ) */
#define _mm_extract_epi64(a, imm) \
    ((long long)__builtin_ia32_pextrq128((a), (imm)))

/* _mm_insert_epi8: insert 8-bit int at lane (PINSRB) */
#define _mm_insert_epi8(a, i, imm) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pinsrb128((a), (i), (imm)))

/* _mm_insert_epi32: insert 32-bit int at lane (PINSRD) */
#define _mm_insert_epi32(a, i, imm) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pinsrd128((a), (i), (imm)))

/* _mm_insert_epi64: insert 64-bit int at lane (PINSRQ) */
#define _mm_insert_epi64(a, i, imm) \
    __CCC_M128I_FROM_BUILTIN(__builtin_ia32_pinsrq128((a), (i), (imm)))

/* === SSE4.1 comparison intrinsics === */

/* _mm_cmpeq_epi64: compare packed 64-bit integers for equality (PCMPEQQ) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cmpeq_epi64(__m128i __a, __m128i __b)
{
    long long *__pa = (long long *)&__a;
    long long *__pb = (long long *)&__b;
    __m128i __r;
    long long *__pr = (long long *)&__r;
    __pr[0] = (__pa[0] == __pb[0]) ? -1LL : 0LL;
    __pr[1] = (__pa[1] == __pb[1]) ? -1LL : 0LL;
    return __r;
}

/* === SSE4.1 blending === */

/* _mm_blendv_epi8: byte-level blend using mask high bits (PBLENDVB) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_blendv_epi8(__m128i __a, __m128i __b, __m128i __mask)
{
    /* For each byte: result = (mask_byte & 0x80) ? b_byte : a_byte */
    unsigned char *__pa = (unsigned char *)&__a;
    unsigned char *__pb = (unsigned char *)&__b;
    unsigned char *__pm = (unsigned char *)&__mask;
    __m128i __r;
    unsigned char *__pr = (unsigned char *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (__pm[__i] & 0x80) ? __pb[__i] : __pa[__i];
    return __r;
}

/* === SSE4.1 min/max signed byte === */

/* `pminsb`/`pmaxsb` are SSE4.1 and have a single-instruction encoding, so
 * these are the primitive itself rather than a lane-at-a-time loop.  The
 * loop form was not merely slow: 16 scalar signed compares per call are a
 * shape the SLP vectorizer can only recover as `pcmpgtb` + `pblendvb` (three
 * instructions on the AVX2 path, more with the loop's own loads and stores
 * before -O2 copies the operand images to the stack).
 *
 * `__builtin_ia32_pminsb128`/`pmaxsb128` are gated on `-msse4.1` by the
 * front end, which is what GCC's own `target("sse4.1")`-attributed header
 * definitions do at the call site. */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_max_epi8(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pmaxsb128((__v16qi)(__a), (__v16qi)(__b));
}

/* _mm_min_epi8: packed signed 8-bit min (PMINSB) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_min_epi8(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pminsb128((__v16qi)(__a), (__v16qi)(__b));
}

/* === SSE4.1 min/max unsigned 32-bit and signed 32-bit === */

/* Dword extrema are the same story one lane width up: `pmaxsd`/`pminsd`/
 * `pmaxud`/`pminud` are SSE4.1 single instructions.  The four-iteration
 * scalar loop these replace measured 48 instructions for `_mm_max_epi32` and
 * 37 for `_mm_min_epu32` at -O2 (GCC 16.2: 2), because a per-lane
 * `cmov` chain has no vector encoding the loop vectorizer will find. */

/* _mm_max_epi32: packed signed 32-bit max (PMAXSD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_max_epi32(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pmaxsd128((__v4si)(__a), (__v4si)(__b));
}

/* _mm_min_epi32: packed signed 32-bit min (PMINSD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_min_epi32(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pminsd128((__v4si)(__a), (__v4si)(__b));
}

/* _mm_max_epu32: packed unsigned 32-bit max (PMAXUD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_max_epu32(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pmaxud128((__v4su)(__a), (__v4su)(__b));
}

/* _mm_min_epu32: packed unsigned 32-bit min (PMINUD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_min_epu32(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pminud128((__v4su)(__a), (__v4su)(__b));
}

/* _mm_max_epu16: packed unsigned 16-bit max (PMAXUW), one instruction. */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_max_epu16(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pmaxuw128((__v8hi)(__a), (__v8hi)(__b));
}

/* _mm_min_epu16: packed unsigned 16-bit min (PMINUW), one instruction. */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_min_epu16(__m128i __a, __m128i __b)
{
    return (__m128i)__builtin_ia32_pminuw128((__v8hi)(__a), (__v8hi)(__b));
}

/* _mm_max_epi16: packed signed 16-bit max - already in SSE2 (PMAXSW)
 * _mm_min_epi16: packed signed 16-bit min - already in SSE2 (PMINSW) */

/* === SSE4.1 zero/sign extension === */

/* _mm_cvtepi8_epi16: sign-extend 8 low bytes to 8 shorts (PMOVSXBW) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepi8_epi16(__m128i __a)
{
    signed char *__pa = (signed char *)&__a;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* _mm_cvtepi8_epi32: sign-extend 4 low bytes to 4 ints (PMOVSXBD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepi8_epi32(__m128i __a)
{
    signed char *__pa = (signed char *)&__a;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* _mm_cvtepu8_epi16: zero-extend 8 low bytes to 8 shorts (PMOVZXBW) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepu8_epi16(__m128i __a)
{
    unsigned char *__pa = (unsigned char *)&__a;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* _mm_cvtepu8_epi32: zero-extend 4 low bytes to 4 ints (PMOVZXBD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepu8_epi32(__m128i __a)
{
    unsigned char *__pa = (unsigned char *)&__a;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* _mm_cvtepi16_epi32: sign-extend 4 low shorts to 4 ints (PMOVSXWD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepi16_epi32(__m128i __a)
{
    short *__pa = (short *)&__a;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* _mm_cvtepu16_epi32: zero-extend 4 low shorts to 4 ints (PMOVZXWD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepu16_epi32(__m128i __a)
{
    unsigned short *__pa = (unsigned short *)&__a;
    __m128i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 4; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* _mm_cvtepi32_epi64: sign-extend 2 low ints to 2 longs (PMOVSXDQ) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepi32_epi64(__m128i __a)
{
    int *__pa = (int *)&__a;
    __m128i __r;
    long long *__pr = (long long *)&__r;
    for (int __i = 0; __i < 2; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* _mm_cvtepu32_epi64: zero-extend 2 low ints to 2 longs (PMOVZXDQ) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_cvtepu32_epi64(__m128i __a)
{
    unsigned int *__pa = (unsigned int *)&__a;
    __m128i __r;
    long long *__pr = (long long *)&__r;
    for (int __i = 0; __i < 2; __i++)
        __pr[__i] = __pa[__i];
    return __r;
}

/* === SSE4.1 test === */

/* _mm_testz_si128: test all zeros (PTEST) - returns 1 if (a & b) == 0 */
static __inline__ int __attribute__((__always_inline__, target("sse4.1")))
_mm_testz_si128(__m128i __a, __m128i __b)
{
    return (__a[0] & __b[0]) == 0 && (__a[1] & __b[1]) == 0;
}

/* _mm_cmpgt_epi64: signed 64-bit greater-than (PCMPGTQ, SSE4.2). A vector
 * comparison yields -1 or 0 per lane, which is the mask form the intrinsic
 * returns. */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.2")))
_mm_cmpgt_epi64(__m128i __a, __m128i __b)
{
    return (__m128i)((__v2di)__a > (__v2di)__b);
}

/* _mm_testc_si128: CF of PTEST, 1 iff (~a & b) == 0 */
static __inline__ int __attribute__((__always_inline__, target("sse4.1")))
_mm_testc_si128(__m128i __a, __m128i __b)
{
    return (~__a[0] & __b[0]) == 0 && (~__a[1] & __b[1]) == 0;
}

/* _mm_testnzc_si128: 1 iff ZF == 0 and CF == 0 */
static __inline__ int __attribute__((__always_inline__, target("sse4.1")))
_mm_testnzc_si128(__m128i __a, __m128i __b)
{
    return !_mm_testz_si128(__a, __b) && !_mm_testc_si128(__a, __b);
}

/* Macro forms, as in GCC 16.2. Each operand is evaluated once. */
#define _mm_test_all_zeros(__m, __v) _mm_testz_si128((__m), (__v))
#define _mm_test_mix_ones_zeros(__m, __v) _mm_testnzc_si128((__m), (__v))
#define _mm_test_all_ones(__v)                                                 \
    __extension__ ({                                                           \
        __m128i __tao_v = (__v);                                               \
        _mm_testc_si128(__tao_v, _mm_cmpeq_epi32(__tao_v, __tao_v));           \
    })

/* === SSE4.1 round (ROUNDPS/ROUNDPD and the scalar forms) ===
 * The scalar forms round only the low element of the second operand, and
 * take the upper elements from the first. Rounding the packed value and
 * merging the low lane gives exactly that, with the same rounding mode. */
#define _mm_round_ss(__d, __v, __m)                                            \
    __extension__ ({                                                           \
        __m128 __rss_d = (__d);                                                \
        _mm_move_ss(__rss_d, _mm_round_ps((__v), (__m)));                      \
    })
#define _mm_round_sd(__d, __v, __m)                                            \
    __extension__ ({                                                           \
        __m128d __rsd_d = (__d);                                               \
        _mm_move_sd(__rsd_d, _mm_round_pd((__v), (__m)));                      \
    })
#define _mm_ceil_ps(__v) _mm_round_ps((__v), _MM_FROUND_CEIL)
#define _mm_floor_ps(__v) _mm_round_ps((__v), _MM_FROUND_FLOOR)
#define _mm_ceil_pd(__v) _mm_round_pd((__v), _MM_FROUND_CEIL)
#define _mm_floor_pd(__v) _mm_round_pd((__v), _MM_FROUND_FLOOR)
#define _mm_ceil_ss(__d, __v) _mm_round_ss((__d), (__v), _MM_FROUND_CEIL)
#define _mm_floor_ss(__d, __v) _mm_round_ss((__d), (__v), _MM_FROUND_FLOOR)
#define _mm_ceil_sd(__d, __v) _mm_round_sd((__d), (__v), _MM_FROUND_CEIL)
#define _mm_floor_sd(__d, __v) _mm_round_sd((__d), (__v), _MM_FROUND_FLOOR)

/* === SSE4.1 multiply === */

/* _mm_mullo_epi32: packed 32-bit multiply low (PMULLD) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_mullo_epi32(__m128i __a, __m128i __b)
{
    return ((__m128i)(__builtin_ia32_pmulld128(__a, __b)));
}

/* === CRC32 intrinsics (SSE4.2) === */

static __inline__ unsigned int __attribute__((__always_inline__, target("sse4.2")))
_mm_crc32_u8(unsigned int __crc, unsigned char __v)
{
    return __builtin_ia32_crc32qi(__crc, __v);
}

static __inline__ unsigned int __attribute__((__always_inline__, target("sse4.2")))
_mm_crc32_u16(unsigned int __crc, unsigned short __v)
{
    return __builtin_ia32_crc32hi(__crc, __v);
}

static __inline__ unsigned int __attribute__((__always_inline__, target("sse4.2")))
_mm_crc32_u32(unsigned int __crc, unsigned int __v)
{
    return __builtin_ia32_crc32si(__crc, __v);
}

static __inline__ unsigned long long __attribute__((__always_inline__, target("sse4.2")))
_mm_crc32_u64(unsigned long long __crc, unsigned long long __v)
{
    return __builtin_ia32_crc32di(__crc, __v);
}

/* === Pack with Unsigned Saturation (SSE4.1) === */

/* Pack 32-bit signed integers from a and b to 16-bit unsigned integers with unsigned saturation */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_packus_epi32(__m128i __a, __m128i __b)
{
    __m128i __r;
    int *__ai = (int *)&__a;
    int *__bi = (int *)&__b;
    unsigned short __rr[8];
    for (int __i = 0; __i < 4; __i++) {
        int __v = __ai[__i];
        if (__v > 65535) __v = 65535;
        if (__v < 0) __v = 0;
        __rr[__i] = (unsigned short)__v;
    }
    for (int __i = 0; __i < 4; __i++) {
        int __v = __bi[__i];
        if (__v > 65535) __v = 65535;
        if (__v < 0) __v = 0;
        __rr[__i + 4] = (unsigned short)__v;
    }
    __builtin_memcpy(&__r, __rr, 16);
    return __r;
}


/* _mm_blend_epi16: blend 16-bit elements by immediate mask (PBLENDW) */
static __inline__ __m128i __attribute__((__always_inline__, target("sse4.1")))
_mm_blend_epi16(__m128i __a, __m128i __b, const int __imm)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m128i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = ((__imm >> __i) & 1) ? __pb[__i] : __pa[__i];
    return __r;
}

/* --- Streaming load (SSE4.1, MOVNTDQA). The source must be 16-byte aligned. --- */

static __inline__ __m128i __attribute__((__always_inline__))
_mm_stream_load_si128(__m128i *__p)
{
    return (__m128i)__builtin_ia32_movntdqa((__v2di *)__p);
}

#endif /* _SMMINTRIN_H_INCLUDED */
