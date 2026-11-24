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

/* `pand`/`pandn` are frontend builtins that lower to one `vpand`/`vpandn`.
 * Expressing them as element-wise initializers (the previous form) leaves the
 * bitwise op to the SLP vectorizer, which does not see through the
 * per-element lane expressions in every context. */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_and_si256(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pand256((__v4di)(__a), (__v4di)(__b));
}

#define _mm256_andnot_si256(__a, __b) \
    (__m256i)__builtin_ia32_andnotsi256(__a, __b)

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
_mm256_add_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_paddw256((__v16hi)(__a), (__v16hi)(__b));
}
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sub_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_psubb256((__v32qi)(__a), (__v32qi)(__b));
}
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sub_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_psubw256((__v16hi)(__a), (__v16hi)(__b));
}

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
/* `_mm256_blend_epi32` selects 32-bit elements by the immediate, exactly like
 * `vblendps` -- the two differ only in the mnemonic and the domain, and both
 * leave unselected elements bit-identical.  The lane loop it used to be spilled
 * both inputs to the stack and wrote the result lane by lane.
 *
 * A macro, not an inline function: the immediate must stay a constant
 * expression at the call site, and a function parameter is not one.
 *
 * AVX2 only (VPBLENDD, VPSHUFD-256).  Declared unconditionally, as GCC 16.2
 * does: both are ISA-gated at codegen (lcccsimd / builtin requirement), so a
 * plain -mavx caller is rejected while a target("avx2") caller is accepted.
 * Pure-C AVX2 wrappers below stay behind __AVX2__: they have no codegen gate,
 * so declaring them under -mavx would emit AVX2 instructions into an AVX-only
 * translation unit (probe fail-open, measured). */
#define _mm256_blend_epi32(__a, __b, __imm) \
    (__CCC_M256I_FROM_BUILTIN( \
        __lccc_simd256_i_vpblendd256((__imm), (__m256i)(__a), (__m256i)(__b))))

#define _mm_blend_epi32(__a, __b, __imm) \
    (__CCC_M128I_FROM_BUILTIN( \
        __lccc_simd128_i_vpblendd128((__imm), (__m128i)(__a), (__m128i)(__b))))

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

/* _mm256_movemask_epi8: VPMOVMSKB - create 32-bit mask from MSBs of bytes.
 *
 * This was a 32-iteration scalar loop that stored the vector to the stack and
 * tested each byte, i.e. 32 branches per call for one instruction.  It is the
 * inner operation of every byte-classification loop (JSON/XML scanning, UTF-8
 * validation, `memchr` overlays), so the loop was also the hottest thing in
 * the function that used it.  `__builtin_ia32_pmovmskb256` is the instruction. */
static __inline__ int __attribute__((__always_inline__))
_mm256_movemask_epi8(__m256i __a)
{
    return __builtin_ia32_pmovmskb256(__a);
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

/* === Element extract and insert of a 256-bit vector ===
 * The element is named by a constant inside the vector: GCC 16.2 refuses
 * `_mm256_extract_epi32(v, 8)` and a selector of the running program (by
 * the 128-bit half the selector would pick), and so do the macros below.
 * They were functions that masked the selector: element 8 was element 0. */
static __inline__ int __attribute__((__always_inline__))
__lccc_mm256_extract_epi8(__m256i __a, int __imm)
{
    unsigned char *__p = (unsigned char *)&__a;
    return (int)__p[__imm & 31];
}
#define _mm256_extract_epi8(__a, __imm)                                          \
    (__LCCC_IMMEDIATE(__imm, 31, "selector must be an integer constant in the range [0, 31]"), \
     __lccc_mm256_extract_epi8((__a), (__imm)))

static __inline__ int __attribute__((__always_inline__))
__lccc_mm256_extract_epi32(__m256i __a, int __imm)
{
    int *__p = (int *)&__a;
    return __p[__imm & 7];
}
#define _mm256_extract_epi32(__a, __imm)                                         \
    (__LCCC_IMMEDIATE(__imm, 7, "selector must be an integer constant in the range [0, 7]"), \
     __lccc_mm256_extract_epi32((__a), (__imm)))

static __inline__ long long __attribute__((__always_inline__))
__lccc_mm256_extract_epi64(__m256i __a, int __imm)
{
    long long *__p = (long long *)&__a;
    return __p[__imm & 3];
}
#define _mm256_extract_epi64(__a, __imm)                                         \
    (__LCCC_IMMEDIATE(__imm, 3, "selector must be an integer constant in the range [0, 3]"), \
     __lccc_mm256_extract_epi64((__a), (__imm)))

static __inline__ __m256i __attribute__((__always_inline__))
__lccc_mm256_insert_epi8(__m256i __a, int __val, int __imm)
{
    unsigned char *__p = (unsigned char *)&__a;
    __p[__imm & 31] = (unsigned char)__val;
    return __a;
}
#define _mm256_insert_epi8(__a, __val, __imm)                                    \
    (__LCCC_IMMEDIATE(__imm, 31, "selector must be an integer constant in the range [0, 31]"), \
     __lccc_mm256_insert_epi8((__a), (__val), (__imm)))

static __inline__ __m256i __attribute__((__always_inline__))
__lccc_mm256_insert_epi32(__m256i __a, int __val, int __imm)
{
    int *__p = (int *)&__a;
    __p[__imm & 7] = __val;
    return __a;
}
#define _mm256_insert_epi32(__a, __val, __imm)                                   \
    (__LCCC_IMMEDIATE(__imm, 7, "selector must be an integer constant in the range [0, 7]"), \
     __lccc_mm256_insert_epi32((__a), (__val), (__imm)))

static __inline__ __m256i __attribute__((__always_inline__))
__lccc_mm256_insert_epi64(__m256i __a, long long __val, int __imm)
{
    long long *__p = (long long *)&__a;
    __p[__imm & 3] = __val;
    return __a;
}
#define _mm256_insert_epi64(__a, __val, __imm)                                   \
    (__LCCC_IMMEDIATE(__imm, 3, "selector must be an integer constant in the range [0, 3]"), \
     __lccc_mm256_insert_epi64((__a), (__val), (__imm)))

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

/* _mm256_max_epi8 / _mm256_min_epi8: packed SIGNED 8-bit extrema
 * (VPMAXSB/VPMINSB).  Unlike the unsigned byte pair above these have no
 * AVX2 spelling in this header at all until now -- `-mavx2` code that GCC
 * compiles failed here with an implicit-declaration error. */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_max_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaxsb256((__v32qi)(__a), (__v32qi)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_min_epi8(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pminsb256((__v32qi)(__a), (__v32qi)(__b));
}

/* _mm256_max_epu32 / _mm256_min_epu32: packed UNSIGNED 32-bit extrema
 * (VPMAXUD/VPMINUD) -- also previously absent. */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_max_epu32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaxud256((__v8su)(__a), (__v8su)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_min_epu32(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pminud256((__v8su)(__a), (__v8su)(__b));
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

/* _mm256_packs_epi16: pack 16-bit to 8-bit with signed saturation within lanes (VPACKSSWB) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_packs_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_packsswb256((__v16hi)(__a), (__v16hi)(__b));
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

#if defined(__AVX2__)
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
#endif /* __AVX2__ */


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

/* _mm256_max_epu16 / _mm256_min_epu16: packed unsigned 16-bit extrema.
 *
 * One `vpmaxuw` / `vpminuw` (AVX2) each, through the same builtins GCC 16.2
 * uses.  Like the other AVX2 wrappers in this header the requirement is
 * deferred to the caller, so a caller without AVX2 is diagnosed rather than
 * handed a sequence that merely looks equivalent.
 */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_max_epu16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaxuw256((__v16hi)(__a), (__v16hi)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_min_epu16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pminuw256((__v16hi)(__a), (__v16hi)(__b));
}

/* _mm256_max_epi16 / _mm256_min_epi16: packed SIGNED 16-bit extrema
 * (VPMAXSW/VPMINSW, AVX2).  These were absent from the header, so `-mavx2`
 * code that GCC compiles failed with an implicit-declaration error. */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_max_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pmaxsw256((__v16hi)(__a), (__v16hi)(__b));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_min_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)__builtin_ia32_pminsw256((__v16hi)(__a), (__v16hi)(__b));
}

/* Everything below is AVX2-only: GCC defines these with target("avx2"), so
 * under -mavx they must not exist. -mavx2 defines __AVX2__; -mavx does not. */
#ifdef __AVX2__

/* === Vector-extension AVX2 completions ===
 * Written with the portable GCC/Clang vector operators instead of per-element
 * loops, so each operator lowers to one native AVX2 instruction (vpaddw,
 * vpsllvd, vpcmpgtb, vpblendvb-equivalent AND/OR) and no lane value takes a
 * memory round trip. Saturation is the standard exact construction: compute the
 * wrapped result and the overflow predicate, then select the saturated bound by
 * the sign of the first operand (signed) or by the carry/borrow (unsigned).
 * Immediate and count operands are reduced to the architectural range before
 * the shift, so no lane ever sees an out-of-range vector shift count. */

typedef unsigned char __lccc_v32qu __attribute__((__vector_size__(32)));
typedef unsigned short __lccc_v16hu __attribute__((__vector_size__(32)));
typedef int __lccc_v8si __attribute__((__vector_size__(32)));
typedef unsigned char __lccc_v16qu __attribute__((__vector_size__(16)));
typedef char __lccc_v16qi __attribute__((__vector_size__(16)));
typedef short __lccc_v8hi __attribute__((__vector_size__(16)));
typedef unsigned short __lccc_v8hu __attribute__((__vector_size__(16)));
typedef int __lccc_v4si __attribute__((__vector_size__(16)));
typedef unsigned int __lccc_v4su __attribute__((__vector_size__(16)));
typedef long long __lccc_v2di __attribute__((__vector_size__(16)));
typedef unsigned long long __lccc_v2du __attribute__((__vector_size__(16)));
typedef long long __lccc_v4di __attribute__((__vector_size__(32)));
typedef unsigned long long __lccc_v4du __attribute__((__vector_size__(32)));
typedef unsigned int __lccc_v8su __attribute__((__vector_size__(32)));

/* --- Saturating add/subtract (VPADDSB/W, VPADDUSB/W, VPSUBSB/W, VPSUBUSB/W) --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_adds_epi8(__m256i __a, __m256i __b)
{
    __v32qi __x = (__v32qi)__a, __y = (__v32qi)__b;
    __v32qi __s = __x + __y;
    /* Signed overflow iff both operands share a sign and the sum does not. */
    __v32qi __ovf = ((__x ^ __s) & (__y ^ __s)) < 0;
    __v32qi __sat = (__x >> 7) ^ 0x7F; /* 0x7F for x >= 0, 0x80 for x < 0 */
    return (__m256i)((__s & ~__ovf) | (__sat & __ovf));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_adds_epi16(__m256i __a, __m256i __b)
{
    __v16hi __x = (__v16hi)__a, __y = (__v16hi)__b;
    __v16hi __s = __x + __y;
    __v16hi __ovf = ((__x ^ __s) & (__y ^ __s)) < 0;
    __v16hi __sat = (__x >> 15) ^ 0x7FFF;
    return (__m256i)((__s & ~__ovf) | (__sat & __ovf));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_subs_epi8(__m256i __a, __m256i __b)
{
    __v32qi __x = (__v32qi)__a, __y = (__v32qi)__b;
    __v32qi __d = __x - __y;
    /* Signed overflow iff the operands differ in sign and the result's sign
     * differs from the minuend. */
    __v32qi __ovf = ((__x ^ __y) & (__x ^ __d)) < 0;
    __v32qi __sat = (__x >> 7) ^ 0x7F;
    return (__m256i)((__d & ~__ovf) | (__sat & __ovf));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_subs_epi16(__m256i __a, __m256i __b)
{
    __v16hi __x = (__v16hi)__a, __y = (__v16hi)__b;
    __v16hi __d = __x - __y;
    __v16hi __ovf = ((__x ^ __y) & (__x ^ __d)) < 0;
    __v16hi __sat = (__x >> 15) ^ 0x7FFF;
    return (__m256i)((__d & ~__ovf) | (__sat & __ovf));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_adds_epu8(__m256i __a, __m256i __b)
{
    __lccc_v32qu __x = (__lccc_v32qu)__a, __y = (__lccc_v32qu)__b;
    __lccc_v32qu __s = __x + __y;
    /* Unsigned overflow iff the wrapped sum is below an operand. */
    __lccc_v32qu __ovf = (__lccc_v32qu)(__s < __x);
    return (__m256i)(__s | __ovf);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_adds_epu16(__m256i __a, __m256i __b)
{
    __lccc_v16hu __x = (__lccc_v16hu)__a, __y = (__lccc_v16hu)__b;
    __lccc_v16hu __s = __x + __y;
    __lccc_v16hu __ovf = (__lccc_v16hu)(__s < __x);
    return (__m256i)(__s | __ovf);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_subs_epu8(__m256i __a, __m256i __b)
{
    __lccc_v32qu __x = (__lccc_v32qu)__a, __y = (__lccc_v32qu)__b;
    __lccc_v32qu __d = __x - __y;
    /* Unsigned borrow iff x < y: the saturated result is zero. */
    __lccc_v32qu __borrow = (__lccc_v32qu)(__x < __y);
    return (__m256i)(__d & ~__borrow);
}

/* --- Rounding average (VPAVGB/W): (a + b + 1) >> 1 without widening.
 * (a|b) - ((a^b)>>1) equals ceil((a+b)/2) for unsigned operands. --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_avg_epu8(__m256i __a, __m256i __b)
{
    __lccc_v32qu __x = (__lccc_v32qu)__a, __y = (__lccc_v32qu)__b;
    return (__m256i)((__x | __y) - ((__x ^ __y) >> 1));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_avg_epu16(__m256i __a, __m256i __b)
{
    __lccc_v16hu __x = (__lccc_v16hu)__a, __y = (__lccc_v16hu)__b;
    return (__m256i)((__x | __y) - ((__x ^ __y) >> 1));
}

/* --- Compare equal for words (VPCMPEQW). The byte form is a frontend builtin. --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_cmpeq_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)((__v16hi)__a == (__v16hi)__b);
}

/* GCC's 128-bit spelling of the AVX2 broadcast; it is an alias, not a new op. */
#define _mm_broadcastsi128_si256(__X) _mm256_broadcastsi128_si256((__X))

/* --- Compare greater-than for words (VPCMPGTW). The byte form
 * _mm256_cmpgt_epi8 is a frontend builtin (X86Pcmpgtb256) and is not redefined
 * here. --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_cmpgt_epi16(__m256i __a, __m256i __b)
{
    return (__m256i)((__v16hi)__a > (__v16hi)__b);
}

/* --- Sign (VPSIGNB/W/D): negate a where b < 0, keep a where b > 0, zero where
 * b == 0. Conditional negate is (x ^ m) - m with m = -1 for negative lanes. --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sign_epi8(__m256i __a, __m256i __b)
{
    __v32qi __x = (__v32qi)__a, __y = (__v32qi)__b;
    __v32qi __m = __y >> 7;
    __v32qi __z = __y == 0;
    return (__m256i)(((__x ^ __m) - __m) & ~__z);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sign_epi16(__m256i __a, __m256i __b)
{
    __v16hi __x = (__v16hi)__a, __y = (__v16hi)__b;
    __v16hi __m = __y >> 15;
    __v16hi __z = __y == 0;
    return (__m256i)(((__x ^ __m) - __m) & ~__z);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sign_epi32(__m256i __a, __m256i __b)
{
    __lccc_v8si __x = (__lccc_v8si)__a, __y = (__lccc_v8si)__b;
    __lccc_v8si __m = __y >> 31;
    __lccc_v8si __z = __y == 0;
    return (__m256i)(((__x ^ __m) - __m) & ~__z);
}

/* --- Byte blend under a mask (VPBLENDVB): select b where the mask's sign bit
 * is set. The sign bit is widened to a full-byte mask by a compare. --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_blendv_epi8(__m256i __a, __m256i __b, __m256i __mask)
{
    __v32qi __m = (__v32qi)__mask < 0;
    return (__m256i)(((__v32qi)__a & ~__m) | ((__v32qi)__b & __m));
}

/* --- Shifts by a scalar count taken from the low 64 bits of an XMM register.
 * A count at or above the element width gives zero (logical) or the sign fill
 * (arithmetic), per the Intel definition. --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sll_epi16(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    if (__c > 15)
        return _mm256_setzero_si256();
    return (__m256i)((__lccc_v16hu)__a << (int)__c);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sll_epi32(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    if (__c > 31)
        return _mm256_setzero_si256();
    return (__m256i)((__lccc_v8su)__a << (int)__c);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sll_epi64(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    if (__c > 63)
        return _mm256_setzero_si256();
    return (__m256i)((__lccc_v4du)__a << (int)__c);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_srl_epi16(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    if (__c > 15)
        return _mm256_setzero_si256();
    return (__m256i)((__lccc_v16hu)__a >> (int)__c);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_srl_epi32(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    if (__c > 31)
        return _mm256_setzero_si256();
    return (__m256i)((__lccc_v8su)__a >> (int)__c);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_srl_epi64(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    if (__c > 63)
        return _mm256_setzero_si256();
    return (__m256i)((__lccc_v4du)__a >> (int)__c);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sra_epi16(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    int __k = __c > 15 ? 15 : (int)__c;
    return (__m256i)((__v16hi)__a >> __k);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sra_epi32(__m256i __a, __m128i __count)
{
    unsigned long long __c = ((__lccc_v2du)__count)[0];
    int __k = __c > 31 ? 31 : (int)__c;
    return (__m256i)((__lccc_v8si)__a >> __k);
}

/* --- Per-element variable shifts (VPSLLV/VPSRLV/VPSRAV). An out-of-range count
 * gives zero (logical) or the sign fill (arithmetic). The count is masked to
 * the element width before the hardware shift, and the out-of-range lanes are
 * then cleared or saturated by a lane mask. --- */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sllv_epi32(__m256i __a, __m256i __count)
{
    __lccc_v8su __k = (__lccc_v8su)__count;
    __lccc_v8su __in = (__lccc_v8su)(__k < 32);
    return (__m256i)(((__lccc_v8su)__a << (__k & 31)) & __in);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_sllv_epi64(__m256i __a, __m256i __count)
{
    __lccc_v4du __k = (__lccc_v4du)__count;
    __lccc_v4du __in = (__lccc_v4du)(__k < 64);
    return (__m256i)(((__lccc_v4du)__a << (__k & 63)) & __in);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_srlv_epi32(__m256i __a, __m256i __count)
{
    __lccc_v8su __k = (__lccc_v8su)__count;
    __lccc_v8su __in = (__lccc_v8su)(__k < 32);
    return (__m256i)(((__lccc_v8su)__a >> (__k & 31)) & __in);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_srlv_epi64(__m256i __a, __m256i __count)
{
    __lccc_v4du __k = (__lccc_v4du)__count;
    __lccc_v4du __in = (__lccc_v4du)(__k < 64);
    return (__m256i)(((__lccc_v4du)__a >> (__k & 63)) & __in);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_srav_epi32(__m256i __a, __m256i __count)
{
    __lccc_v8su __k = (__lccc_v8su)__count;
    __lccc_v8su __big = (__lccc_v8su)(__k > 31);
    __lccc_v8su __kk = (__k & ~__big) | (31 & __big);
    return (__m256i)((__lccc_v8si)__a >> (__lccc_v8si)__kk);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_sllv_epi32(__m128i __a, __m128i __count)
{
    __lccc_v4su __k = (__lccc_v4su)__count;
    __lccc_v4su __in = (__lccc_v4su)(__k < 32);
    return (__m128i)(((__lccc_v4su)__a << (__k & 31)) & __in);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_sllv_epi64(__m128i __a, __m128i __count)
{
    __lccc_v2du __k = (__lccc_v2du)__count;
    __lccc_v2du __in = (__lccc_v2du)(__k < 64);
    return (__m128i)(((__lccc_v2du)__a << (__k & 63)) & __in);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_srlv_epi32(__m128i __a, __m128i __count)
{
    __lccc_v4su __k = (__lccc_v4su)__count;
    __lccc_v4su __in = (__lccc_v4su)(__k < 32);
    return (__m128i)(((__lccc_v4su)__a >> (__k & 31)) & __in);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_srlv_epi64(__m128i __a, __m128i __count)
{
    __lccc_v2du __k = (__lccc_v2du)__count;
    __lccc_v2du __in = (__lccc_v2du)(__k < 64);
    return (__m128i)(((__lccc_v2du)__a >> (__k & 63)) & __in);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_srav_epi32(__m128i __a, __m128i __count)
{
    __lccc_v4su __k = (__lccc_v4su)__count;
    __lccc_v4su __big = (__lccc_v4su)(__k > 31);
    __lccc_v4su __kk = (__k & ~__big) | (31 & __big);
    return (__m128i)((__lccc_v4si)__a >> (__lccc_v4si)__kk);
}

/* --- 128-bit broadcasts of the low element (VBROADCASTSS). ---
 * _mm_broadcastsd_pd is AVX1 and lives in avxintrin.h. */

static __inline__ __m128 __attribute__((__always_inline__))
_mm_broadcastss_ps(__m128 __a)
{
    return _mm_shuffle_ps(__a, __a, 0x00);
}

/* --- Streaming load and integer sign-bit-masked moves (AVX2). ---
 * VMOVNTDQA is the non-temporal load (SSE4.1 for 128 bits, AVX2 for 256).
 * VPMASKMOV{D,Q} keep the element-size contract of the float forms above. */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_stream_load_si256(__m256i const *__p)
{
    return (__m256i)__builtin_ia32_movntdqa256((__v4di *)__p);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_maskload_epi32(int const *__p, __m256i __m)
{
    return (__m256i)__builtin_ia32_maskloadd256((const __v8si *)__p, (__v8si)__m);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_maskload_epi64(long long const *__p, __m256i __m)
{
    return (__m256i)__builtin_ia32_maskloadq256((const __v4di *)__p, (__v4di)__m);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_maskstore_epi32(int *__p, __m256i __m, __m256i __a)
{
    __builtin_ia32_maskstored256((__v8si *)__p, (__v8si)__m, (__v8si)__a);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_maskstore_epi64(long long *__p, __m256i __m, __m256i __a)
{
    __builtin_ia32_maskstoreq256((__v4di *)__p, (__v4di)__m, (__v4di)__a);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_maskload_epi32(int const *__p, __m128i __m)
{
    return (__m128i)__builtin_ia32_maskloadd((const __v4si *)__p, (__v4si)__m);
}

static __inline__ __m128i __attribute__((__always_inline__))
_mm_maskload_epi64(long long const *__p, __m128i __m)
{
    return (__m128i)__builtin_ia32_maskloadq((const __v2di *)__p, (__v2di)__m);
}

static __inline__ void __attribute__((__always_inline__))
_mm_maskstore_epi32(int *__p, __m128i __m, __m128i __a)
{
    __builtin_ia32_maskstored((__v4si *)__p, (__v4si)__m, (__v4si)__a);
}

static __inline__ void __attribute__((__always_inline__))
_mm_maskstore_epi64(long long *__p, __m128i __m, __m128i __a)
{
    __builtin_ia32_maskstoreq((__v2di *)__p, (__v2di)__m, (__v2di)__a);
}

#endif /* __AVX2__ */

#endif /* _AVX2INTRIN_H_INCLUDED */
