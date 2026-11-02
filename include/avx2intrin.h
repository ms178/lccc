/* CCC compiler bundled avx2intrin.h - AVX2 integer intrinsics */
#ifndef _AVX2INTRIN_H_INCLUDED
#define _AVX2INTRIN_H_INCLUDED

#include <avxintrin.h>

/* === Set === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set1_epi8(char __b)
{
    unsigned char __ub = (unsigned char)__b;
    long long __q = (long long)__ub;
    __q |= __q << 8;
    __q |= __q << 16;
    __q |= __q << 32;
    return (__m256i){  __q, __q, __q, __q  };
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set1_epi16(short __w)
{
    unsigned short __uw = (unsigned short)__w;
    long long __q = (long long)__uw;
    __q |= __q << 16;
    __q |= __q << 32;
    return (__m256i){  __q, __q, __q, __q  };
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set1_epi32(int __i)
{
    long long __q = (long long)(unsigned int)__i
                  | ((unsigned long long)(unsigned int)__i << 32);
    return (__m256i){  __q, __q, __q, __q  };
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set1_epi64x(long long __q)
{
    return (__m256i){  __q, __q, __q, __q  };
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_setr_epi8(
    char __b0,  char __b1,  char __b2,  char __b3,
    char __b4,  char __b5,  char __b6,  char __b7,
    char __b8,  char __b9,  char __b10, char __b11,
    char __b12, char __b13, char __b14, char __b15,
    char __b16, char __b17, char __b18, char __b19,
    char __b20, char __b21, char __b22, char __b23,
    char __b24, char __b25, char __b26, char __b27,
    char __b28, char __b29, char __b30, char __b31)
{
    __m256i __r;
    unsigned char *__p = (unsigned char *)&__r;
    __p[0]  = (unsigned char)__b0;  __p[1]  = (unsigned char)__b1;
    __p[2]  = (unsigned char)__b2;  __p[3]  = (unsigned char)__b3;
    __p[4]  = (unsigned char)__b4;  __p[5]  = (unsigned char)__b5;
    __p[6]  = (unsigned char)__b6;  __p[7]  = (unsigned char)__b7;
    __p[8]  = (unsigned char)__b8;  __p[9]  = (unsigned char)__b9;
    __p[10] = (unsigned char)__b10; __p[11] = (unsigned char)__b11;
    __p[12] = (unsigned char)__b12; __p[13] = (unsigned char)__b13;
    __p[14] = (unsigned char)__b14; __p[15] = (unsigned char)__b15;
    __p[16] = (unsigned char)__b16; __p[17] = (unsigned char)__b17;
    __p[18] = (unsigned char)__b18; __p[19] = (unsigned char)__b19;
    __p[20] = (unsigned char)__b20; __p[21] = (unsigned char)__b21;
    __p[22] = (unsigned char)__b22; __p[23] = (unsigned char)__b23;
    __p[24] = (unsigned char)__b24; __p[25] = (unsigned char)__b25;
    __p[26] = (unsigned char)__b26; __p[27] = (unsigned char)__b27;
    __p[28] = (unsigned char)__b28; __p[29] = (unsigned char)__b29;
    __p[30] = (unsigned char)__b30; __p[31] = (unsigned char)__b31;
    return __r;
}

/* === Bitwise === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_and_si256(__m256i __a, __m256i __b)
{
    return (__m256i){  __a[0] & __b[0],
                        __a[1] & __b[1],
                        __a[2] & __b[2],
                        __a[3] & __b[3]  };
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_andnot_si256(__m256i __a, __m256i __b)
{
    return (__m256i){  ~__a[0] & __b[0],
                        ~__a[1] & __b[1],
                        ~__a[2] & __b[2],
                        ~__a[3] & __b[3]  };
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_or_si256(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_por256((__v4di)(__a), (__v4di)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_xor_si256(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pxor256((__v4di)(__a), (__v4di)(__b));
}

/* === Shift === */

#define _mm256_slli_epi32(__a, __count) \
    ((__m256i)__builtin_ia32_pslldi256((__v8si)(__a), (int)(__count)))

#define _mm256_srli_epi32(__a, __count) \
    ((__m256i)__builtin_ia32_psrldi256((__v8si)(__a), (int)(__count)))

#define _mm256_slli_epi64(__a, __count) \
    ((__m256i)__builtin_ia32_psllqi256((__v4di)(__a), (int)(__count)))

#define _mm256_srli_epi64(__a, __count) \
    ((__m256i)__builtin_ia32_psrlqi256((__v4di)(__a), (int)(__count)))

/* === Compare / Min / Max === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_max_epu8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaxub256((__v32qi)(__a), (__v32qi)(__b));
}

/* === Shuffle === */

/* _mm256_shuffle_epi8: VPSHUFB - byte shuffle within 128-bit lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_shuffle_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pshufb256((__v32qi)(__a), (__v32qi)(__b));
}

/* === Extract === */

/* _mm256_extracti128_si256: extract 128-bit lane (imm must be 0 or 1) */
static __inline__ __m128i __attribute__((__always_inline__))
_mm256_extracti128_si256(__m256i __a, int __imm)
{
    if (__imm & 1)
        return (__m128i){  __a[2], __a[3]  };
    else
        return (__m128i){  __a[0], __a[1]  };
}

/* === Add / Sub === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_add_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_paddb256((__v32qi)(__a), (__v32qi)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_add_epi16(__m256i a, __m256i b) { __m256i r; unsigned short *x=(unsigned short*)&a,*y=(unsigned short*)&b,*z=(unsigned short*)&r; for(int i=0;i<16;i++)z[i]=x[i]+y[i]; return r; }
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sub_epi8(__m256i a, __m256i b) { __m256i r; unsigned char *x=(unsigned char*)&a,*y=(unsigned char*)&b,*z=(unsigned char*)&r; for(int i=0;i<32;i++)z[i]=x[i]-y[i]; return r; }
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sub_epi16(__m256i a, __m256i b) { __m256i r; unsigned short *x=(unsigned short*)&a,*y=(unsigned short*)&b,*z=(unsigned short*)&r; for(int i=0;i<16;i++)z[i]=x[i]-y[i]; return r; }

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_add_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_paddd256((__v8si)(__a), (__v8si)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_add_epi64(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_paddq256((__v4di)(__a), (__v4di)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sub_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_psubd256((__v8si)(__a), (__v8si)(__b));
}

/* === Compare === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_cmpeq_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pcmpeqb256((__v32qi)(__a), (__v32qi)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_cmpeq_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pcmpeqd256((__v8si)(__a), (__v8si)(__b));
}

/* === Permute === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_permute2x128_si256(__m256i __a, __m256i __b, int __imm)
{
    int __sel_lo = __imm & 0x3;
    int __sel_hi = (__imm >> 4) & 0x3;
    long long __result[4];

    /* Select low 128 bits */
    switch (__sel_lo) {
        case 0: __result[0] = __a[0]; __result[1] = __a[1]; break;
        case 1: __result[0] = __a[2]; __result[1] = __a[3]; break;
        case 2: __result[0] = __b[0]; __result[1] = __b[1]; break;
        case 3: __result[0] = __b[2]; __result[1] = __b[3]; break;
    }
    if (__imm & 0x08) { __result[0] = 0; __result[1] = 0; }

    /* Select high 128 bits */
    switch (__sel_hi) {
        case 0: __result[2] = __a[0]; __result[3] = __a[1]; break;
        case 1: __result[2] = __a[2]; __result[3] = __a[3]; break;
        case 2: __result[2] = __b[0]; __result[3] = __b[1]; break;
        case 3: __result[2] = __b[2]; __result[3] = __b[3]; break;
    }
    if (__imm & 0x80) { __result[2] = 0; __result[3] = 0; }

    return (__m256i){  __result[0], __result[1], __result[2], __result[3]  };
}

/* === Set (non-broadcast) === */

/* _mm256_set_epi32: set 8 ints (high-to-low order like Intel) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set_epi32(int __e7, int __e6, int __e5, int __e4,
                 int __e3, int __e2, int __e1, int __e0)
{
    __m256i __r;
    int *__p = (int *)&__r;
    __p[0] = __e0; __p[1] = __e1; __p[2] = __e2; __p[3] = __e3;
    __p[4] = __e4; __p[5] = __e5; __p[6] = __e6; __p[7] = __e7;
    return __r;
}

/* _mm256_set_epi8: set 32 bytes (high-to-low order like Intel) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set_epi8(
    char __b31, char __b30, char __b29, char __b28,
    char __b27, char __b26, char __b25, char __b24,
    char __b23, char __b22, char __b21, char __b20,
    char __b19, char __b18, char __b17, char __b16,
    char __b15, char __b14, char __b13, char __b12,
    char __b11, char __b10, char __b9,  char __b8,
    char __b7,  char __b6,  char __b5,  char __b4,
    char __b3,  char __b2,  char __b1,  char __b0)
{
    __m256i __r;
    unsigned char *__p = (unsigned char *)&__r;
    __p[0]  = (unsigned char)__b0;  __p[1]  = (unsigned char)__b1;
    __p[2]  = (unsigned char)__b2;  __p[3]  = (unsigned char)__b3;
    __p[4]  = (unsigned char)__b4;  __p[5]  = (unsigned char)__b5;
    __p[6]  = (unsigned char)__b6;  __p[7]  = (unsigned char)__b7;
    __p[8]  = (unsigned char)__b8;  __p[9]  = (unsigned char)__b9;
    __p[10] = (unsigned char)__b10; __p[11] = (unsigned char)__b11;
    __p[12] = (unsigned char)__b12; __p[13] = (unsigned char)__b13;
    __p[14] = (unsigned char)__b14; __p[15] = (unsigned char)__b15;
    __p[16] = (unsigned char)__b16; __p[17] = (unsigned char)__b17;
    __p[18] = (unsigned char)__b18; __p[19] = (unsigned char)__b19;
    __p[20] = (unsigned char)__b20; __p[21] = (unsigned char)__b21;
    __p[22] = (unsigned char)__b22; __p[23] = (unsigned char)__b23;
    __p[24] = (unsigned char)__b24; __p[25] = (unsigned char)__b25;
    __p[26] = (unsigned char)__b26; __p[27] = (unsigned char)__b27;
    __p[28] = (unsigned char)__b28; __p[29] = (unsigned char)__b29;
    __p[30] = (unsigned char)__b30; __p[31] = (unsigned char)__b31;
    return __r;
}

/* === Insert === */

/* _mm256_inserti128_si256: insert 128-bit lane into 256-bit register */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_inserti128_si256(__m256i __a, __m128i __b, int __imm)
{
    __m256i __r = __a;
    if (__imm & 1) {
        __r[2] = __b[0];
        __r[3] = __b[1];
    } else {
        __r[0] = __b[0];
        __r[1] = __b[1];
    }
    return __r;
}

/* === Blend === */

/* _mm256_blend_epi32: VPBLENDD - blend 32-bit elements by immediate mask */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_blend_epi32(__m256i __a, __m256i __b, int __imm)
{
    int *__pa = (int *)&__a;
    int *__pb = (int *)&__b;
    __m256i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = (__imm & (1 << __i)) ? __pb[__i] : __pa[__i];
    return __r;
}

/* === Shuffle (32-bit) === */

/* _mm256_shuffle_epi32: VPSHUFD - shuffle 32-bit ints within 128-bit lanes */
#define _mm256_shuffle_epi32(__a, __imm) \
    ((__m256i)__builtin_ia32_pshufd256((__v8si)(__a), (int)(__imm)))

/* === Shift (16-bit) === */

/* _mm256_slli_epi16: VPSLLW - shift 16-bit ints left by immediate */
#define _mm256_slli_epi16(__a, __count) \
    ((__m256i)__builtin_ia32_psllwi256((__v16hi)(__a), (int)(__count)))

/* _mm256_srli_epi16: VPSRLW - shift 16-bit ints right by immediate */
#define _mm256_srli_epi16(__a, __count) \
    ((__m256i)__builtin_ia32_psrlwi256((__v16hi)(__a), (int)(__count)))

/* === Movemask === */

/* _mm256_movemask_epi8: VPMOVMSKB - create 32-bit mask from MSBs of bytes */
static __inline__ int __attribute__((__always_inline__))
_mm256_movemask_epi8(__m256i __a)
{
    unsigned char *__pa = (unsigned char *)&__a;
    int __mask = 0;
    for (int __i = 0; __i < 32; __i++) {
        if (__pa[__i] & 0x80)
            __mask |= (1 << __i);
    }
    return __mask;
}

/* === Permute (64-bit) === */

/* _mm256_permute4x64_epi64: VPERMQ - permute 64-bit elements across lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_permute4x64_epi64(__m256i __a, int __imm)
{
    __m256i __r;
    __r[0] = __a[(__imm >> 0) & 3];
    __r[1] = __a[(__imm >> 2) & 3];
    __r[2] = __a[(__imm >> 4) & 3];
    __r[3] = __a[(__imm >> 6) & 3];
    return __r;
}

/* _mm256_permutevar8x32_epi32: VPERMD - permute 32-bit ints by variable index */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_permutevar8x32_epi32(__m256i __a, __m256i __idx)
{
    return (__m256i)__builtin_ia32_permvarsi256((__v8si)(__a), (__v8si)(__idx));
}

/* === Unpack / Interleave === */

/* _mm256_unpacklo_epi8: VPUNPCKLBW - interleave low bytes within 128-bit lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpacklo_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpcklbw256((__v32qi)(__a), (__v32qi)(__b));
}

/* _mm256_unpackhi_epi8: VPUNPCKHBW - interleave high bytes within 128-bit lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpackhi_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpckhbw256((__v32qi)(__a), (__v32qi)(__b));
}

/* _mm256_unpacklo_epi16: VPUNPCKLWD - interleave low 16-bit ints within lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpacklo_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpcklwd256((__v16hi)(__a), (__v16hi)(__b));
}

/* _mm256_unpackhi_epi16: VPUNPCKHWD - interleave high 16-bit ints within lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpackhi_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpckhwd256((__v16hi)(__a), (__v16hi)(__b));
}

/* _mm256_unpacklo_epi32: VPUNPCKLDQ - interleave low 32-bit ints within lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpacklo_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpckldq256((__v8si)(__a), (__v8si)(__b));
}

/* _mm256_unpackhi_epi32: VPUNPCKHDQ - interleave high 32-bit ints within lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpackhi_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpckhdq256((__v8si)(__a), (__v8si)(__b));
}

/* _mm256_unpacklo_epi64: VPUNPCKLQDQ - interleave low 64-bit ints within lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpacklo_epi64(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpcklqdq256((__v4di)(__a), (__v4di)(__b));
}

/* _mm256_unpackhi_epi64: VPUNPCKHQDQ - interleave high 64-bit ints within lanes */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_unpackhi_epi64(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_punpckhqdq256((__v4di)(__a), (__v4di)(__b));
}

/* === Additional intrinsics for CPython HACL Blake2b SIMD256 === */

/* _mm256_set_epi64x: set 4 x 64-bit integers (high-to-low) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set_epi64x(long long __e3, long long __e2, long long __e1, long long __e0)
{
    return (__m256i){  __e0, __e1, __e2, __e3  };
}

/* _mm256_set_m128i: combine two __m128i into __m256i (lo, hi) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set_m128i(__m128i __hi, __m128i __lo)
{
    __m256i __r;
    long long *__p = (long long *)&__r;
    long long *__lo_p = (long long *)&__lo;
    long long *__hi_p = (long long *)&__hi;
    __p[0] = __lo_p[0]; __p[1] = __lo_p[1];
    __p[2] = __hi_p[0]; __p[3] = __hi_p[1];
    return __r;
}

/* _mm256_cmpeq_epi64: compare 64-bit integers for equality */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_cmpeq_epi64(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pcmpeqq256((__v4di)(__a), (__v4di)(__b));
}

/* _mm256_cmpgt_epi32: compare signed 32-bit integers for greater-than */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_cmpgt_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pcmpgtd256((__v8si)(__a), (__v8si)(__b));
}

/* _mm256_cmpgt_epi64: compare signed 64-bit integers for greater-than */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_cmpgt_epi64(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pcmpgtq256((__v4di)(__a), (__v4di)(__b));
}

/* _mm256_extract_epi8: extract 8-bit integer */
static __inline__ int __attribute__((__always_inline__))
_mm256_extract_epi8(__m256i __a, int __imm)
{
    unsigned char *__p = (unsigned char *)&__a;
    return (int)__p[__imm & 31];
}

/* _mm256_extract_epi32: extract 32-bit integer */
static __inline__ int __attribute__((__always_inline__))
_mm256_extract_epi32(__m256i __a, int __imm)
{
    int *__p = (int *)&__a;
    return __p[__imm & 7];
}

/* _mm256_extract_epi64: extract 64-bit integer */
static __inline__ long long __attribute__((__always_inline__))
_mm256_extract_epi64(__m256i __a, int __imm)
{
    long long *__p = (long long *)&__a;
    return __p[__imm & 3];
}

/* _mm256_insert_epi8: insert 8-bit integer */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_insert_epi8(__m256i __a, int __val, int __imm)
{
    unsigned char *__p = (unsigned char *)&__a;
    __p[__imm & 31] = (unsigned char)__val;
    return __a;
}

/* _mm256_insert_epi32: insert 32-bit integer */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_insert_epi32(__m256i __a, int __val, int __imm)
{
    int *__p = (int *)&__a;
    __p[__imm & 7] = __val;
    return __a;
}

/* _mm256_insert_epi64: insert 64-bit integer */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_insert_epi64(__m256i __a, long long __val, int __imm)
{
    long long *__p = (long long *)&__a;
    __p[__imm & 3] = __val;
    return __a;
}

/* _mm256_sub_epi64: subtract 64-bit integers */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sub_epi64(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_psubq256((__v4di)(__a), (__v4di)(__b));
}

/* _mm256_mul_epu32: multiply unsigned 32-bit integers, produce 64-bit results */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_mul_epu32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmuludq256((__v8si)(__a), (__v8si)(__b));
}

/* _mm256_mullo_epi32: multiply signed 32-bit integers, keep low 32 bits */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_mullo_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmulld256((__v8si)(__a), (__v8si)(__b));
}

/* _mm256_slli_si256: byte shift left within 128-bit lanes */
#define _mm256_slli_si256(__a, __imm) \
    ((__m256i)__builtin_ia32_pslldqi256((__v4di)(__a), (((int)(__imm)) * 8)))

/* _mm256_srli_si256: byte shift right within 128-bit lanes */
#define _mm256_srli_si256(__a, __imm) \
    ((__m256i)__builtin_ia32_psrldqi256((__v4di)(__a), (((int)(__imm)) * 8)))

/* === Native AVX2 Sign / Zero Extension === */

void *__lccc_avx2_pmovsxbw256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepi8_epi16(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovsxbw256(__a));
}

void *__lccc_avx2_pmovsxbd256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepi8_epi32(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovsxbd256(__a));
}

void *__lccc_avx2_pmovsxbq256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepi8_epi64(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovsxbq256(__a));
}

void *__lccc_avx2_pmovsxwd256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepi16_epi32(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovsxwd256(__a));
}

void *__lccc_avx2_pmovsxwq256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepi16_epi64(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovsxwq256(__a));
}

void *__lccc_avx2_pmovsxdq256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepi32_epi64(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovsxdq256(__a));
}

void *__lccc_avx2_pmovzxbw256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepu8_epi16(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovzxbw256(__a));
}

void *__lccc_avx2_pmovzxbd256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepu8_epi32(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovzxbd256(__a));
}

void *__lccc_avx2_pmovzxbq256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepu8_epi64(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovzxbq256(__a));
}

void *__lccc_avx2_pmovzxwd256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepu16_epi32(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovzxwd256(__a));
}

void *__lccc_avx2_pmovzxwq256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepu16_epi64(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovzxwq256(__a));
}

void *__lccc_avx2_pmovzxdq256(__m128i);
static __inline__ __m256i __attribute__((__always_inline__, __target__("avx2")))
_mm256_cvtepu32_epi64(__m128i __a)
{
    return __CCC_M256I_FROM_BUILTIN(__lccc_avx2_pmovzxdq256(__a));
}

/* === Multiply-Add === */

/* _mm256_madd_epi16: multiply signed 16-bit, horizontally add adjacent pairs -> 32-bit (VPMADDWD) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_madd_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaddwd256((__v16hi)(__a), (__v16hi)(__b));
}

/* _mm256_maddubs_epi16: multiply unsigned*signed 8-bit, hadd pairs -> 16-bit with saturation (VPMADDUBSW) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_maddubs_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaddubsw256((__v32qi)(__a), (__v32qi)(__b));
}

/* === Horizontal Add === */

/* _mm256_hadd_epi16: horizontal add adjacent pairs of 16-bit (VPHADDW) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_hadd_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_phaddw256((__v16hi)(__a), (__v16hi)(__b));
}

/* _mm256_hadd_epi32: horizontal add adjacent pairs of 32-bit (VPHADDD) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_hadd_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_phaddd256((__v8si)(__a), (__v8si)(__b));
}

/* === Multiply (16-bit) === */

/* _mm256_mullo_epi16: multiply 16-bit ints, keep low 16 bits (VPMULLW) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_mullo_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmullw256((__v16hi)(__a), (__v16hi)(__b));
}

/* _mm256_mulhi_epi16: multiply signed 16-bit ints, keep high 16 bits (VPMULHW) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_mulhi_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmulhw256((__v16hi)(__a), (__v16hi)(__b));
}

/* === Absolute value === */

/* _mm256_abs_epi8: VPABSB */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_abs_epi8(__m256i __a)
{
    return (__m256i)__builtin_ia32_pabsb256((__v32qi)(__a));
}

/* _mm256_abs_epi16: VPABSW */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_abs_epi16(__m256i __a)
{
    return (__m256i)__builtin_ia32_pabsw256((__v16hi)(__a));
}

/* _mm256_abs_epi32: VPABSD */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_abs_epi32(__m256i __a)
{
    return (__m256i)__builtin_ia32_pabsd256((__v8si)(__a));
}

/* === Min / Max (additional) === */

/* _mm256_min_epu8: minimum of unsigned 8-bit ints (VPMINUB) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_min_epu8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pminub256((__v32qi)(__a), (__v32qi)(__b));
}

/* _mm256_max_epi32: maximum of signed 32-bit ints (VPMAXSD) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_max_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaxsd256((__v8si)(__a), (__v8si)(__b));
}

/* _mm256_min_epi32: minimum of signed 32-bit ints (VPMINSD) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_min_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pminsd256((__v8si)(__a), (__v8si)(__b));
}

/* === Pack === */

/* _mm256_packs_epi32: pack 32-bit to 16-bit with signed saturation within lanes (VPACKSSDW) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_packs_epi32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_packssdw256((__v8si)(__a), (__v8si)(__b));
}

/* _mm256_packus_epi16: pack 16-bit to 8-bit with unsigned saturation within lanes (VPACKUSWB) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_packus_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_packuswb256((__v16hi)(__a), (__v16hi)(__b));
}

/* === Arithmetic shift === */

/* _mm256_srai_epi32: arithmetic shift right 32-bit (VPSRAD) */
#define _mm256_srai_epi32(__a, __count) \
    ((__m256i)__builtin_ia32_psradi256((__v8si)(__a), (int)(__count)))

/* _mm256_srai_epi16: arithmetic shift right 16-bit (VPSRAW) */
#define _mm256_srai_epi16(__a, __count) \
    ((__m256i)__builtin_ia32_psrawi256((__v16hi)(__a), (int)(__count)))

/* === Gather === */

/* _mm256_i32gather_epi32: gather 32-bit ints using 32-bit indices (VPGATHERDD) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_i32gather_epi32(int const *__base, __m256i __index, int __scale)
{
    int *__pi = (int *)&__index;
    __m256i __r;
    int *__pr = (int *)&__r;
    const unsigned char *__b = (const unsigned char *)__base;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = *(int *)(__b + (long long)__pi[__i] * __scale);
    return __r;
}


/* === Cast / move between 128-bit and 256-bit === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_zextsi128_si256(__m128i __a)
{
    return _mm256_castsi128_si256(__a);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_broadcastsi128_si256(__m128i __a)
{
    return _mm256_inserti128_si256(_mm256_castsi128_si256(__a), __a, 1);
}

/* Set 8 x 32-bit lanes in low-to-high order. */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_setr_epi32(int __e0, int __e1, int __e2, int __e3,
                  int __e4, int __e5, int __e6, int __e7)
{
    __m256i __r;
    int *__p = (int *)&__r;
    __p[0] = __e0; __p[1] = __e1; __p[2] = __e2; __p[3] = __e3;
    __p[4] = __e4; __p[5] = __e5; __p[6] = __e6; __p[7] = __e7;
    return __r;
}

/* Two SSE2 halves preserve AVX2 PSADBW lane semantics. */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sad_epu8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_psadbw256((__v32qi)(__a), (__v32qi)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_subs_epu16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_psubusw256((__v16hi)(__a), (__v16hi)(__b));
}

#endif /* _AVX2INTRIN_H_INCLUDED */
