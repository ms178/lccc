/* CCC compiler bundled avx512fintrin.h - AVX-512 Foundation intrinsics */
#ifndef _AVX512FINTRIN_H_INCLUDED
#define _AVX512FINTRIN_H_INCLUDED

#include <avx2intrin.h>

/* AVX-512 512-bit vector types */
typedef struct __attribute__((__aligned__(64))) {
    long long __val[8];
} __m512i;

typedef struct __attribute__((__aligned__(64))) {
    double __val[8];
} __m512d;

typedef struct __attribute__((__aligned__(64))) {
    float __val[16];
} __m512;

/* Unaligned variants */
typedef struct __attribute__((__aligned__(1))) {
    long long __val[8];
} __m512i_u;

/* AVX-512 mask types */
typedef unsigned char __mmask8;
typedef unsigned short __mmask16;
typedef unsigned int __mmask32;
typedef unsigned long long __mmask64;

/* === Load / Store === */

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_loadu_si512(void const *__p)
{
    __m512i __r;
    __builtin_memcpy(&__r, __p, sizeof(__r));
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm512_storeu_si512(void *__p, __m512i __a)
{
    __builtin_memcpy(__p, &__a, sizeof(__a));
}

/* === Set === */

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_setzero_si512(void)
{
    return (__m512i){ { 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL } };
}

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set1_epi64(long long __q)
{
    return (__m512i){ { __q, __q, __q, __q, __q, __q, __q, __q } };
}

/* === Arithmetic === */

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_add_epi64(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] + __b.__val[0],
                        __a.__val[1] + __b.__val[1],
                        __a.__val[2] + __b.__val[2],
                        __a.__val[3] + __b.__val[3],
                        __a.__val[4] + __b.__val[4],
                        __a.__val[5] + __b.__val[5],
                        __a.__val[6] + __b.__val[6],
                        __a.__val[7] + __b.__val[7] } };
}

/* === Population count === */

/* _mm512_popcnt_epi64: population count for each 64-bit element */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_popcnt_epi64(__m512i __a)
{
    __m512i __r;
    for (int __i = 0; __i < 8; __i++) {
        unsigned long long __v = (unsigned long long)__a.__val[__i];
        int __cnt = 0;
        while (__v) {
            __cnt++;
            __v &= __v - 1;
        }
        __r.__val[__i] = __cnt;
    }
    return __r;
}

/* === Reduce === */

/* _mm512_reduce_add_epi64: horizontal sum of all 64-bit elements */
static __inline__ long long __attribute__((__always_inline__))
_mm512_reduce_add_epi64(__m512i __a)
{
    return __a.__val[0] + __a.__val[1] + __a.__val[2] + __a.__val[3]
         + __a.__val[4] + __a.__val[5] + __a.__val[6] + __a.__val[7];
}

/* === Float Load / Store === */

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_loadu_ps(void const *__p)
{
    __m512 __r;
    const float *__fp = (const float *)__p;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __fp[__i];
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm512_storeu_ps(void *__p, __m512 __a)
{
    float *__fp = (float *)__p;
    for (int __i = 0; __i < 16; __i++)
        __fp[__i] = __a.__val[__i];
}

/* === Float Set === */

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_setzero_ps(void)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = 0.0f;
    return __r;
}

/* === Float Arithmetic === */

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_add_ps(__m512 __a, __m512 __b)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __r;
}

static __inline__ __m512 __attribute__((__always_inline__))
_mm512_mul_ps(__m512 __a, __m512 __b)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i];
    return __r;
}

/* _mm512_fmadd_ps: a*b + c (single-precision, 512-bit) */
static __inline__ __m512 __attribute__((__always_inline__))
_mm512_fmadd_ps(__m512 __a, __m512 __b, __m512 __c)
{
    __m512 __r;
    for (int __i = 0; __i < 16; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i] + __c.__val[__i];
    return __r;
}

/* === Float Reduce === */

/* _mm512_reduce_add_ps: horizontal sum of all 16 float elements */
static __inline__ float __attribute__((__always_inline__))
_mm512_reduce_add_ps(__m512 __a)
{
    float __sum = 0.0f;
    for (int __i = 0; __i < 16; __i++)
        __sum += __a.__val[__i];
    return __sum;
}

/* === Integer Arithmetic (32-bit) === */

/* _mm512_add_epi32: add packed 32-bit integers */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_add_epi32(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] + __pb[__i];
    return __r;
}

/* _mm512_sub_epi32: subtract packed 32-bit integers */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_sub_epi32(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] - __pb[__i];
    return __r;
}

/* _mm512_mullo_epi32: multiply 32-bit ints, keep low 32 bits */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_mullo_epi32(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] * __pb[__i];
    return __r;
}

/* === Bitwise === */

/* _mm512_and_si512: bitwise AND */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_and_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] & __b.__val[0],
                        __a.__val[1] & __b.__val[1],
                        __a.__val[2] & __b.__val[2],
                        __a.__val[3] & __b.__val[3],
                        __a.__val[4] & __b.__val[4],
                        __a.__val[5] & __b.__val[5],
                        __a.__val[6] & __b.__val[6],
                        __a.__val[7] & __b.__val[7] } };
}

/* _mm512_or_si512: bitwise OR */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_or_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] | __b.__val[0],
                        __a.__val[1] | __b.__val[1],
                        __a.__val[2] | __b.__val[2],
                        __a.__val[3] | __b.__val[3],
                        __a.__val[4] | __b.__val[4],
                        __a.__val[5] | __b.__val[5],
                        __a.__val[6] | __b.__val[6],
                        __a.__val[7] | __b.__val[7] } };
}

/* _mm512_xor_si512: bitwise XOR */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_xor_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] ^ __b.__val[0],
                        __a.__val[1] ^ __b.__val[1],
                        __a.__val[2] ^ __b.__val[2],
                        __a.__val[3] ^ __b.__val[3],
                        __a.__val[4] ^ __b.__val[4],
                        __a.__val[5] ^ __b.__val[5],
                        __a.__val[6] ^ __b.__val[6],
                        __a.__val[7] ^ __b.__val[7] } };
}

/* _mm512_andnot_si512: bitwise AND-NOT (~a & b) */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_andnot_si512(__m512i __a, __m512i __b)
{
    return (__m512i){ { ~__a.__val[0] & __b.__val[0],
                        ~__a.__val[1] & __b.__val[1],
                        ~__a.__val[2] & __b.__val[2],
                        ~__a.__val[3] & __b.__val[3],
                        ~__a.__val[4] & __b.__val[4],
                        ~__a.__val[5] & __b.__val[5],
                        ~__a.__val[6] & __b.__val[6],
                        ~__a.__val[7] & __b.__val[7] } };
}

/* === Set (additional) === */

/* _mm512_set1_epi32: broadcast 32-bit integer */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set1_epi32(int __i)
{
    long long __q = (long long)(unsigned int)__i
                  | ((long long)(unsigned int)__i << 32);
    return (__m512i){ { __q, __q, __q, __q, __q, __q, __q, __q } };
}

/* _mm512_set1_epi8: broadcast 8-bit integer */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set1_epi8(char __b)
{
    unsigned char __ub = (unsigned char)__b;
    long long __q = (long long)__ub;
    __q |= __q << 8;
    __q |= __q << 16;
    __q |= __q << 32;
    return (__m512i){ { __q, __q, __q, __q, __q, __q, __q, __q } };
}

/* === Extract === */

/* _mm512_extracti64x4_epi64: extract 256-bit lane from 512-bit register */
static __inline__ __m256i __attribute__((__always_inline__))
_mm512_extracti64x4_epi64(__m512i __a, int __imm)
{
    if (__imm & 1)
        return (__m256i){ { __a.__val[4], __a.__val[5], __a.__val[6], __a.__val[7] } };
    else
        return (__m256i){ { __a.__val[0], __a.__val[1], __a.__val[2], __a.__val[3] } };
}

/* _mm512_extracti32x4_epi32: extract 128-bit lane from 512-bit register */
static __inline__ __m128i __attribute__((__always_inline__))
_mm512_extracti32x4_epi32(__m512i __a, int __imm)
{
    int __lane = __imm & 3;
    return (__m128i){ { __a.__val[__lane * 2], __a.__val[__lane * 2 + 1] } };
}

/* === Conversion / Sign Extension (512-bit) === */

/* _mm512_cvtepi8_epi16: sign-extend 32 packed 8-bit to 32 packed 16-bit (AVX-512BW) */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_cvtepi8_epi16(__m256i __a)
{
    signed char *__pa = (signed char *)&__a;
    __m512i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 32; __i++)
        __pr[__i] = (short)__pa[__i];
    return __r;
}

/* _mm512_cvtepi8_epi32: sign-extend 16 packed 8-bit to 16 packed 32-bit */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_cvtepi8_epi32(__m128i __a)
{
    signed char *__pa = (signed char *)&__a;
    __m512i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (int)__pa[__i];
    return __r;
}

/* _mm512_cvtepi16_epi32: sign-extend 16 packed 16-bit to 16 packed 32-bit */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_cvtepi16_epi32(__m256i __a)
{
    short *__pa = (short *)&__a;
    __m512i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (int)__pa[__i];
    return __r;
}

/* === Multiply-Add (512-bit) === */

/* _mm512_madd_epi16: multiply signed 16-bit, hadd adjacent pairs -> 32-bit (AVX-512BW) */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_madd_epi16(__m512i __a, __m512i __b)
{
    short *__pa = (short *)&__a;
    short *__pb = (short *)&__b;
    __m512i __r;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = (int)__pa[__i * 2] * (int)__pb[__i * 2]
                   + (int)__pa[__i * 2 + 1] * (int)__pb[__i * 2 + 1];
    return __r;
}

/* === Reduce (32-bit) === */

/* _mm512_reduce_add_epi32: horizontal sum of all 32-bit elements */
static __inline__ int __attribute__((__always_inline__))
_mm512_reduce_add_epi32(__m512i __a)
{
    int *__p = (int *)&__a;
    int __sum = 0;
    for (int __i = 0; __i < 16; __i++)
        __sum += __p[__i];
    return __sum;
}

/* === Subtract (64-bit) === */

/* _mm512_sub_epi64: subtract packed 64-bit integers */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_sub_epi64(__m512i __a, __m512i __b)
{
    return (__m512i){ { __a.__val[0] - __b.__val[0],
                        __a.__val[1] - __b.__val[1],
                        __a.__val[2] - __b.__val[2],
                        __a.__val[3] - __b.__val[3],
                        __a.__val[4] - __b.__val[4],
                        __a.__val[5] - __b.__val[5],
                        __a.__val[6] - __b.__val[6],
                        __a.__val[7] - __b.__val[7] } };
}

/* === Shift === */

/* _mm512_slli_epi32: shift 32-bit integers left */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_slli_epi32(__m512i __a, unsigned int __count)
{
    if (__count > 31) return _mm512_setzero_si512();
    unsigned int *__pa = (unsigned int *)&__a;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] << __count;
    return __r;
}

/* _mm512_srli_epi32: shift 32-bit integers right */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_srli_epi32(__m512i __a, unsigned int __count)
{
    if (__count > 31) return _mm512_setzero_si512();
    unsigned int *__pa = (unsigned int *)&__a;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i = 0; __i < 16; __i++)
        __pr[__i] = __pa[__i] >> __count;
    return __r;
}

/* _mm512_slli_epi64: shift 64-bit integers left */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_slli_epi64(__m512i __a, unsigned int __count)
{
    if (__count > 63) return _mm512_setzero_si512();
    unsigned long long *__pa = (unsigned long long *)&__a;
    __m512i __r;
    unsigned long long *__pr = (unsigned long long *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i] << __count;
    return __r;
}

/* _mm512_srli_epi64: shift 64-bit integers right */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_srli_epi64(__m512i __a, unsigned int __count)
{
    if (__count > 63) return _mm512_setzero_si512();
    unsigned long long *__pa = (unsigned long long *)&__a;
    __m512i __r;
    unsigned long long *__pr = (unsigned long long *)&__r;
    for (int __i = 0; __i < 8; __i++)
        __pr[__i] = __pa[__i] >> __count;
    return __r;
}

/* === Compare === */

/* _mm512_cmpeq_epi32_mask: compare 32-bit ints for equality, return mask */
static __inline__ __mmask16 __attribute__((__always_inline__))
_mm512_cmpeq_epi32_mask(__m512i __a, __m512i __b)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pb = (unsigned int *)&__b;
    __mmask16 __mask = 0;
    for (int __i = 0; __i < 16; __i++)
        if (__pa[__i] == __pb[__i])
            __mask |= (1u << __i);
    return __mask;
}

/* === Insert === */

/* _mm512_inserti64x4: insert 256-bit lane into 512-bit register */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_inserti64x4(__m512i __a, __m256i __b, int __imm)
{
    __m512i __r = __a;
    if (__imm & 1) {
        __r.__val[4] = __b.__val[0]; __r.__val[5] = __b.__val[1];
        __r.__val[6] = __b.__val[2]; __r.__val[7] = __b.__val[3];
    } else {
        __r.__val[0] = __b.__val[0]; __r.__val[1] = __b.__val[1];
        __r.__val[2] = __b.__val[2]; __r.__val[3] = __b.__val[3];
    }
    return __r;
}

/* === Broadcast === */

/* _mm512_broadcastsi128_si512: broadcast 128-bit to all 4 lanes */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_broadcastsi128_si512(__m128i __a)
{
    return (__m512i){ { __a.__val[0], __a.__val[1],
                        __a.__val[0], __a.__val[1],
                        __a.__val[0], __a.__val[1],
                        __a.__val[0], __a.__val[1] } };
}


/* === Added for zlib-ng and completeness: missing AVX-512 intrinsics (v1) === */

/* _mm512_castsi128_si512: zero-extend 128 -> 512 (low lane) */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_castsi128_si512(__m128i __a)
{
    __m512i __r = _mm512_setzero_si512();
    __r.__val[0] = __a.__val[0];
    __r.__val[1] = __a.__val[1];
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_zextsi128_si512(__m128i __a)
{
    return _mm512_castsi128_si512(__a);
}

/* _mm512_castsi512_si256: extract low 256 */
static __inline__ __m256i __attribute__((__always_inline__))
_mm512_castsi512_si256(__m512i __a)
{
    return (__m256i){ { __a.__val[0], __a.__val[1], __a.__val[2], __a.__val[3] } };
}

/* _mm512_set_epi32: 16 ints high-to-low */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set_epi32(int __e15, int __e14, int __e13, int __e12,
                 int __e11, int __e10, int __e9, int __e8,
                 int __e7, int __e6, int __e5, int __e4,
                 int __e3, int __e2, int __e1, int __e0)
{
    __m512i __r;
    int *__p = (int *)&__r;
    __p[0]=__e0; __p[1]=__e1; __p[2]=__e2; __p[3]=__e3;
    __p[4]=__e4; __p[5]=__e5; __p[6]=__e6; __p[7]=__e7;
    __p[8]=__e8; __p[9]=__e9; __p[10]=__e10; __p[11]=__e11;
    __p[12]=__e12; __p[13]=__e13; __p[14]=__e14; __p[15]=__e15;
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_setr_epi32(int __e0, int __e1, int __e2, int __e3,
                  int __e4, int __e5, int __e6, int __e7,
                  int __e8, int __e9, int __e10, int __e11,
                  int __e12, int __e13, int __e14, int __e15)
{
    return _mm512_set_epi32(__e15,__e14,__e13,__e12,__e11,__e10,__e9,__e8,__e7,__e6,__e5,__e4,__e3,__e2,__e1,__e0);
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set4_epi32(int __d, int __c, int __b, int __a)
{
    return _mm512_set_epi32(__d,__c,__b,__a, __d,__c,__b,__a, __d,__c,__b,__a, __d,__c,__b,__a);
}

/* _mm512_set_epi8: 64 bytes high-to-low */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set_epi8(char __q63, char __q62, char __q61, char __q60,
                char __q59, char __q58, char __q57, char __q56,
                char __q55, char __q54, char __q53, char __q52,
                char __q51, char __q50, char __q49, char __q48,
                char __q47, char __q46, char __q45, char __q44,
                char __q43, char __q42, char __q41, char __q40,
                char __q39, char __q38, char __q37, char __q36,
                char __q35, char __q34, char __q33, char __q32,
                char __q31, char __q30, char __q29, char __q28,
                char __q27, char __q26, char __q25, char __q24,
                char __q23, char __q22, char __q21, char __q20,
                char __q19, char __q18, char __q17, char __q16,
                char __q15, char __q14, char __q13, char __q12,
                char __q11, char __q10, char __q09, char __q08,
                char __q07, char __q06, char __q05, char __q04,
                char __q03, char __q02, char __q01, char __q00)
{
    __m512i __r;
    unsigned char *__p = (unsigned char *)&__r;
    __p[0]=__q00; __p[1]=__q01; __p[2]=__q02; __p[3]=__q03; __p[4]=__q04; __p[5]=__q05; __p[6]=__q06; __p[7]=__q07;
    __p[8]=__q08; __p[9]=__q09; __p[10]=__q10; __p[11]=__q11; __p[12]=__q12; __p[13]=__q13; __p[14]=__q14; __p[15]=__q15;
    __p[16]=__q16; __p[17]=__q17; __p[18]=__q18; __p[19]=__q19; __p[20]=__q20; __p[21]=__q21; __p[22]=__q22; __p[23]=__q23;
    __p[24]=__q24; __p[25]=__q25; __p[26]=__q26; __p[27]=__q27; __p[28]=__q28; __p[29]=__q29; __p[30]=__q30; __p[31]=__q31;
    __p[32]=__q32; __p[33]=__q33; __p[34]=__q34; __p[35]=__q35; __p[36]=__q36; __p[37]=__q37; __p[38]=__q38; __p[39]=__q39;
    __p[40]=__q40; __p[41]=__q41; __p[42]=__q42; __p[43]=__q43; __p[44]=__q44; __p[45]=__q45; __p[46]=__q46; __p[47]=__q47;
    __p[48]=__q48; __p[49]=__q49; __p[50]=__q50; __p[51]=__q51; __p[52]=__q52; __p[53]=__q53; __p[54]=__q54; __p[55]=__q55;
    __p[56]=__q56; __p[57]=__q57; __p[58]=__q58; __p[59]=__q59; __p[60]=__q60; __p[61]=__q61; __p[62]=__q62; __p[63]=__q63;
    return __r;
}

/* _mm512_set1_epi16: broadcast 16-bit */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set1_epi16(short __w)
{
    unsigned short __v = (unsigned short)__w;
    long long __q = (long long)__v | ((long long)__v << 16);
    __q |= __q << 32;
    return (__m512i){ { __q, __q, __q, __q, __q, __q, __q, __q } };
}

/* _mm512_sad_epu8 */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_sad_epu8(__m512i __a, __m512i __b)
{
    unsigned char *__pa = (unsigned char *)&__a;
    unsigned char *__pb = (unsigned char *)&__b;
    __m512i __r = _mm512_setzero_si512();
    unsigned long long *__pr = (unsigned long long *)&__r;
    for (int __blk = 0; __blk < 8; __blk++) {
        unsigned int __sum = 0;
        for (int __j = 0; __j < 8; __j++) {
            unsigned char __av = __pa[__blk*8+__j];
            unsigned char __bv = __pb[__blk*8+__j];
            __sum += (__av > __bv) ? (__av - __bv) : (__bv - __av);
        }
        __pr[__blk] = __sum;
    }
    return __r;
}

/* _mm512_maddubs_epi16 */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_maddubs_epi16(__m512i __a, __m512i __b)
{
    unsigned char *__pa = (unsigned char *)&__a;
    signed char *__pb = (signed char *)&__b;
    __m512i __r;
    short *__pr = (short *)&__r;
    for (int __i = 0; __i < 32; __i++) {
        int __x = (int)__pa[2*__i] * (int)__pb[2*__i] + (int)__pa[2*__i+1] * (int)__pb[2*__i+1];
        if (__x > 32767) __x = 32767;
        if (__x < -32768) __x = -32768;
        __pr[__i] = (short)__x;
    }
    return __r;
}

/* _mm512_dpbusd_epi32 */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_dpbusd_epi32(__m512i __a, __m512i __b, __m512i __c)
{
    unsigned char *__pb = (unsigned char *)&__b;
    signed char *__pc = (signed char *)&__c;
    int *__pa = (int *)&__a;
    __m512i __r = __a;
    int *__pr = (int *)&__r;
    for (int __i = 0; __i < 16; __i++) {
        int __sum = 0;
        for (int __k = 0; __k < 4; __k++) __sum += (int)__pb[__i*4+__k] * (int)__pc[__i*4+__k];
        __pr[__i] = __pa[__i] + __sum;
    }
    return __r;
}

/* _mm512_clmulepi64_epi128 */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_clmulepi64_epi128(__m512i __a, __m512i __b, const int __imm)
{
    unsigned long long *__pa = (unsigned long long *)&__a;
    unsigned long long *__pb = (unsigned long long *)&__b;
    __m512i __r = _mm512_setzero_si512();
    unsigned long long *__pr = (unsigned long long *)&__r;
    for (int __lane = 0; __lane < 4; __lane++) {
        int __aidx = __lane*2 + ((__imm & 0x01) ? 1:0);
        int __bidx = __lane*2 + ((__imm & 0x10) ? 1:0);
        unsigned long long __av = __pa[__aidx];
        unsigned long long __bv = __pb[__bidx];
        __uint128_t __res = 0;
        for (int __b = 0; __b < 64; __b++) if ((__bv>>__b)&1) __res ^= (__uint128_t)__av << __b;
        __pr[__lane*2] = (unsigned long long)__res;
        __pr[__lane*2+1] = (unsigned long long)(__res>>64);
    }
    return __r;
}

/* _mm512_permutexvar_epi32 */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_permutexvar_epi32(__m512i __idx, __m512i __a)
{
    unsigned int *__pa = (unsigned int *)&__a;
    unsigned int *__pi = (unsigned int *)&__idx;
    __m512i __r;
    unsigned int *__pr = (unsigned int *)&__r;
    for (int __i=0;__i<16;__i++) __pr[__i]=__pa[__pi[__i]&15];
    return __r;
}

/* _mm512_ternarylogic */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_ternarylogic_epi32(__m512i __a, __m512i __b, __m512i __c, int __imm)
{
    unsigned int *__pa=(unsigned int*)&__a, *__pb=(unsigned int*)&__b, *__pc=(unsigned int*)&__c;
    __m512i __r; unsigned int *__pr=(unsigned int*)&__r;
    for(int __i=0;__i<16;__i++){
        unsigned int __av=__pa[__i],__bv=__pb[__i],__cv=__pc[__i],__res=0;
        for(int __bit=0;__bit<32;__bit++){
            int __a_bit=(__av>>__bit)&1, __b_bit=(__bv>>__bit)&1, __c_bit=(__cv>>__bit)&1;
            int __idx = (__a_bit<<2)|(__b_bit<<1)|__c_bit;
            int __out = (__imm>>__idx)&1;
            __res |= (unsigned int)__out << __bit;
        }
        __pr[__i]=__res;
    }
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_ternarylogic_epi64(__m512i __a, __m512i __b, __m512i __c, int __imm)
{
    unsigned long long *__pa=(unsigned long long*)&__a, *__pb=(unsigned long long*)&__b, *__pc=(unsigned long long*)&__c;
    __m512i __r; unsigned long long *__pr=(unsigned long long*)&__r;
    for(int __i=0;__i<8;__i++){
        unsigned long long __av=__pa[__i],__bv=__pb[__i],__cv=__pc[__i],__res=0;
        for(int __bit=0;__bit<64;__bit++){
            int __a_bit=(__av>>__bit)&1, __b_bit=(__bv>>__bit)&1, __c_bit=(__cv>>__bit)&1;
            int __idx = (__a_bit<<2)|(__b_bit<<1)|__c_bit;
            int __out = (__imm>>__idx)&1;
            __res |= (unsigned long long)__out << __bit;
        }
        __pr[__i]=__res;
    }
    return __r;
}

/* masked loads/stores */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_maskz_loadu_epi8(__mmask64 __k, void const *__p)
{
    __m512i __r = _mm512_setzero_si512();
    unsigned char *__dst=(unsigned char*)&__r;
    unsigned char const *__src=(unsigned char const*)__p;
    for(int __i=0;__i<64;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_maskz_loadu_epi16(__mmask32 __k, void const *__p)
{
    __m512i __r=_mm512_setzero_si512();
    unsigned short *__dst=(unsigned short*)&__r;
    unsigned short const *__src=(unsigned short const*)__p;
    for(int __i=0;__i<32;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
    return __r;
}
static __inline__ void __attribute__((__always_inline__))
_mm512_mask_storeu_epi8(void *__p, __mmask64 __k, __m512i __a)
{
    unsigned char *__dst=(unsigned char*)__p;
    unsigned char *__src=(unsigned char*)&__a;
    for(int __i=0;__i<64;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
}
static __inline__ void __attribute__((__always_inline__))
_mm512_mask_storeu_epi16(void *__p, __mmask32 __k, __m512i __a)
{
    unsigned short *__dst=(unsigned short*)__p;
    unsigned short *__src=(unsigned short*)&__a;
    for(int __i=0;__i<32;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
}

/* maskz inserts/extracts */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_maskz_inserti64x2(__mmask8 __k, __m512i __a, __m256i __b, int __imm)
{
    __m512i __r = (__k &1) ? _mm512_inserti64x4(__a,__b,__imm) : __a;
    if ((__k & 0xFF) ==0) return _mm512_setzero_si512();
    return __r;
}
static __inline__ __m128i __attribute__((__always_inline__))
_mm512_maskz_extracti32x4_epi32(__mmask8 __k, __m512i __a, int __imm)
{
    __m128i __r = _mm512_extracti32x4_epi32(__a,__imm);
    if(__k==0) { __m128i __z={ {0,0} }; return __z; }
    unsigned int *__p=(unsigned int*)&__r;
    for(int __i=0;__i<4;__i++) if(!((__k>>__i)&1)) __p[__i]=0;
    return __r;
}
static __inline__ __m256i __attribute__((__always_inline__))
_mm512_maskz_extracti64x4_epi64(__mmask8 __k, __m512i __a, int __imm)
{
    __m256i __r = _mm512_extracti64x4_epi64(__a,__imm);
    if(__k==0){ __m256i __z={ {0,0,0,0} }; return __z; }
    unsigned long long *__p=(unsigned long long*)&__r;
    for(int __i=0;__i<4;__i++) if(!((__k>>__i)&1)) __p[__i]=0;
    return __r;
}

/* inserts */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_inserti32x4(__m512i __a, __m128i __b, int __imm)
{
    __m512i __r=__a;
    int __lane = __imm &3;
    unsigned long long *__pr=(unsigned long long*)&__r;
    __pr[__lane*2]=__b.__val[0];
    __pr[__lane*2+1]=__b.__val[1];
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_inserti64x2(__m512i __a, __m128i __b, int __imm)
{
    __m512i __r=__a;
    if(__imm &1){ __r.__val[4]=__b.__val[0]; __r.__val[5]=__b.__val[1];}
    else{ __r.__val[0]=__b.__val[0]; __r.__val[1]=__b.__val[1];}
    return __r;
}
static __inline__ __m128i __attribute__((__always_inline__))
_mm512_extracti64x2_epi64(__m512i __a, int __imm)
{
    if(__imm &1) return (__m128i){ { __a.__val[2], __a.__val[3] } };
    else return (__m128i){ { __a.__val[0], __a.__val[1] } };
}

/* cmpeq masks */
static __inline__ __mmask64 __attribute__((__always_inline__))
_mm512_cmpeq_epu8_mask(__m512i __a, __m512i __b)
{
    unsigned char *__pa=(unsigned char*)&__a, *__pb=(unsigned char*)&__b;
    __mmask64 __m=0;
    for(int __i=0;__i<64;__i++) if(__pa[__i]==__pb[__i]) __m|=(__mmask64)1<<__i;
    return __m;
}
static __inline__ __mmask16 __attribute__((__always_inline__))
_mm_cmpeq_epu8_mask(__m128i __a, __m128i __b)
{
    unsigned char *__pa=(unsigned char*)&__a, *__pb=(unsigned char*)&__b;
    __mmask16 __m=0;
    for(int __i=0;__i<16;__i++) if(__pa[__i]==__pb[__i]) __m|=(1<<__i);
    return __m;
}
static __inline__ __mmask16 __attribute__((__always_inline__))
_mm_cmp_epi8_mask(__m128i __a, __m128i __b, int __imm)
{
    unsigned char *__pa=(unsigned char*)&__a, *__pb=(unsigned char*)&__b;
    __mmask16 __m=0;
    signed char *__psa=(signed char*)&__a, *__psb=(signed char*)&__b;
    for(int __i=0;__i<16;__i++){
        int __eq = (__pa[__i]==__pb[__i]);
        int __lt = (__psa[__i] < __psb[__i]);
        int __res=0;
        switch(__imm &7){ case 0: __res=__eq; break; case 1: __res=__lt; break; default: __res=__eq; break; }
        if(__res) __m|=(1<<__i);
    }
    return __m;
}
static __inline__ unsigned int __attribute__((__always_inline__))
_mm512_reduce_add_epu32(__m512i __a)
{
    unsigned int *__p=(unsigned int*)&__a;
    unsigned int __s=0;
    for(int __i=0;__i<16;__i++) __s+=__p[__i];
    return __s;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_maskz_maddubs_epi16(__mmask32 __k, __m512i __a, __m512i __b)
{
    __m512i __r=_mm512_setzero_si512();
    unsigned char *__pa=(unsigned char*)&__a, *__pb=(unsigned char*)&__b;
    short *__pr=(short*)&__r;
    for(int __i=0;__i<32;__i++){
        if((__k>>__i)&1){
            int __x=(int)__pa[2*__i]*(int)(signed char)__pb[2*__i] + (int)__pa[2*__i+1]*(int)(signed char)__pb[2*__i+1];
            if(__x>32767) __x=32767; if(__x<-32768) __x=-32768;
            __pr[__i]=(short)__x;
        } else __pr[__i]=0;
    }
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_maskz_set1_epi16(__mmask32 __k, short __a)
{
    __m512i __r=_mm512_setzero_si512();
    short *__pr=(short*)&__r;
    for(int __i=0;__i<32;__i++) if((__k>>__i)&1) __pr[__i]=__a;
    return __r;
}

/* AVX2 masked helpers */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_maskz_loadu_epi8(__mmask32 __k, void const *__p)
{
    __m256i __r; for(int __i=0;__i<4;__i++) __r.__val[__i]=0;
    unsigned char *__dst=(unsigned char*)&__r, *__src=(unsigned char*)__p;
    for(int __i=0;__i<32;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
    return __r;
}
static __inline__ void __attribute__((__always_inline__))
_mm256_mask_storeu_epi8(void *__p, __mmask32 __k, __m256i __a)
{
    unsigned char *__dst=(unsigned char*)__p, *__src=(unsigned char*)&__a;
    for(int __i=0;__i<32;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
}

/* 128 masked */
static __inline__ __m128i __attribute__((__always_inline__))
_mm_maskz_loadu_epi8(__mmask16 __k, void const *__p)
{
    __m128i __r={ {0,0} };
    unsigned char *__dst=(unsigned char*)&__r, *__src=(unsigned char*)__p;
    for(int __i=0;__i<16;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
    return __r;
}
static __inline__ void __attribute__((__always_inline__))
_mm_mask_storeu_epi8(void *__p, __mmask16 __k, __m128i __a)
{
    unsigned char *__dst=(unsigned char*)__p, *__src=(unsigned char*)&__a;
    for(int __i=0;__i<16;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
}
static __inline__ __m128i __attribute__((__always_inline__))
_mm_mask_shuffle_epi8(__m128i __src, __mmask16 __k, __m128i __a, __m128i __b)
{
    unsigned char *__pa=(unsigned char*)&__a, *__pb=(unsigned char*)&__b, *__ps=(unsigned char*)&__src;
    unsigned char *__pr; __m128i __r; __pr=(unsigned char*)&__r;
    for(int __i=0;__i<16;__i++){
        if((__k>>__i)&1) {
            signed char __idx=(signed char)__pb[__i];
            __pr[__i]= (__idx<0)?0:__pa[__idx &15];
        } else __pr[__i]=__ps[__i];
    }
    return __r;
}
static __inline__ __m128i __attribute__((__always_inline__))
_mm_ternarylogic_epi64(__m128i __a, __m128i __b, __m128i __c, int __imm)
{
    unsigned long long *__pa=(unsigned long long*)&__a, *__pb=(unsigned long long*)&__b, *__pc=(unsigned long long*)&__c;
    __m128i __r; unsigned long long *__pr=(unsigned long long*)&__r;
    for(int __i=0;__i<2;__i++){
        unsigned long long __av=__pa[__i],__bv=__pb[__i],__cv=__pc[__i],__res=0;
        for(int __bit=0;__bit<64;__bit++){
            int __a_bit=(__av>>__bit)&1, __b_bit=(__bv>>__bit)&1, __c_bit=(__cv>>__bit)&1;
            int __idx=(__a_bit<<2)|(__b_bit<<1)|__c_bit;
            int __out=(__imm>>__idx)&1;
            __res|=(unsigned long long)__out<<__bit;
        }
        __pr[__i]=__res;
    }
    return __r;
}

/* === Additional AVX-512BW completeness === */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_add_epi8(__m512i __a, __m512i __b)
{
    unsigned char *__pa=(unsigned char*)&__a, *__pb=(unsigned char*)&__b;
    __m512i __r; unsigned char *__pr=(unsigned char*)&__r;
    for(int __i=0;__i<64;__i++) __pr[__i]=__pa[__i]+__pb[__i];
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_sub_epi8(__m512i __a, __m512i __b)
{
    unsigned char *__pa=(unsigned char*)&__a, *__pb=(unsigned char*)&__b;
    __m512i __r; unsigned char *__pr=(unsigned char*)&__r;
    for(int __i=0;__i<64;__i++) __pr[__i]=__pa[__i]-__pb[__i];
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_add_epi16(__m512i __a, __m512i __b)
{
    unsigned short *__pa=(unsigned short*)&__a, *__pb=(unsigned short*)&__b;
    __m512i __r; unsigned short *__pr=(unsigned short*)&__r;
    for(int __i=0;__i<32;__i++) __pr[__i]=__pa[__i]+__pb[__i];
    return __r;
}
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_sub_epi16(__m512i __a, __m512i __b)
{
    unsigned short *__pa=(unsigned short*)&__a, *__pb=(unsigned short*)&__b;
    __m512i __r; unsigned short *__pr=(unsigned short*)&__r;
    for(int __i=0;__i<32;__i++) __pr[__i]=__pa[__i]-__pb[__i];
    return __r;
}
/* xgetbv */
static __inline__ unsigned long long __attribute__((__always_inline__))
_xgetbv(unsigned int __imm)
{
    (void)__imm; return 0;
}


/* _MM_CMPINT_* predicates for _mm_cmp_epi* _mask */
#define _MM_CMPINT_EQ    0
#define _MM_CMPINT_LT    1
#define _MM_CMPINT_LE    2
#define _MM_CMPINT_FALSE 3
#define _MM_CMPINT_NE    4
#define _MM_CMPINT_NLT   5
#define _MM_CMPINT_NLE   6
#define _MM_CMPINT_TRUE  7

#endif /* _AVX512FINTRIN_H_INCLUDED */
