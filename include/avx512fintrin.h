/* CCC compiler bundled avx512fintrin.h - AVX-512 Foundation intrinsics */
#ifndef _AVX512FINTRIN_H_INCLUDED
#define _AVX512FINTRIN_H_INCLUDED

#include <avx2intrin.h>

/* AVX-512 512-bit vector types */
/* Unaligned variants */
/* AVX-512 mask types */
typedef unsigned char __mmask8;
typedef unsigned short __mmask16;
typedef unsigned int __mmask32;
typedef unsigned long long __mmask64;

/* === Load / Store === */



/* === Set === */

static __inline__ __m512i __attribute__((__always_inline__))
_mm512_setzero_si512(void)
{
    return (__m512i){ { 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL, 0LL } };
}


/* === Arithmetic === */


/* === Population count === */

/* _mm512_popcnt_epi64: population count for each 64-bit element */

/* === Reduce === */

/* _mm512_reduce_add_epi64: horizontal sum of all 64-bit elements */

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

/* _mm512_sub_epi32: subtract packed 32-bit integers */

/* _mm512_mullo_epi32: multiply 32-bit ints, keep low 32 bits */

/* === Bitwise === */

/* _mm512_and_si512: bitwise AND */

/* _mm512_or_si512: bitwise OR */

/* _mm512_xor_si512: bitwise XOR */

/* _mm512_andnot_si512: bitwise AND-NOT (~a & b) */

/* === Set (additional) === */

/* _mm512_set1_epi32: broadcast 32-bit integer */

/* _mm512_set1_epi8: broadcast 8-bit integer */

/* === Extract === */

/* _mm512_extracti64x4_epi64: extract 256-bit lane from 512-bit register */

/* _mm512_extracti32x4_epi32: extract 128-bit lane from 512-bit register */

/* === Conversion / Sign Extension (512-bit) === */

/* _mm512_cvtepi8_epi16: sign-extend 32 packed 8-bit to 32 packed 16-bit (AVX-512BW) */

/* _mm512_cvtepi8_epi32: sign-extend 16 packed 8-bit to 16 packed 32-bit */

/* _mm512_cvtepi16_epi32: sign-extend 16 packed 16-bit to 16 packed 32-bit */

/* === Multiply-Add (512-bit) === */

/* _mm512_madd_epi16: multiply signed 16-bit, hadd adjacent pairs -> 32-bit (AVX-512BW) */

/* === Reduce (32-bit) === */

/* _mm512_reduce_add_epi32: horizontal sum of all 32-bit elements */

/* === Subtract (64-bit) === */

/* _mm512_sub_epi64: subtract packed 64-bit integers */

/* === Shift === */

/* _mm512_slli_epi32: shift 32-bit integers left */

/* _mm512_srli_epi32: shift 32-bit integers right */

/* _mm512_slli_epi64: shift 64-bit integers left */

/* _mm512_srli_epi64: shift 64-bit integers right */

/* === Compare === */

/* _mm512_cmpeq_epi32_mask: compare 32-bit ints for equality, return mask */

/* === Insert === */

/* _mm512_inserti64x4: insert 256-bit lane into 512-bit register */

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

/* _mm512_castsi512_si256: extract low 256 */

/* _mm512_set_epi64: 8 int64s high-to-low */
static __inline__ __m512i __attribute__((__always_inline__))
_mm512_set_epi64(long long __e7, long long __e6, long long __e5, long long __e4,
                 long long __e3, long long __e2, long long __e1, long long __e0)
{
    return (__m512i){ { __e0, __e1, __e2, __e3, __e4, __e5, __e6, __e7 } };
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

/* _mm512_sad_epu8 */

/* _mm512_maddubs_epi16 */

/* _mm512_dpbusd_epi32 */

/* _mm512_clmulepi64_epi128 */

/* _mm512_permutexvar_epi32 */

/* _mm512_ternarylogic */

/* masked loads/stores */
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
_mm512_mask_storeu_epi16(void *__p, __mmask32 __k, __m512i __a)
{
    unsigned short *__dst=(unsigned short*)__p;
    unsigned short *__src=(unsigned short*)&__a;
    for(int __i=0;__i<32;__i++) if((__k>>__i)&1) __dst[__i]=__src[__i];
}

/* maskz inserts/extracts */

/* inserts */

/* cmpeq masks */

/* AVX2 masked helpers */

/* 128 masked */
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

/* === Additional AVX-512BW completeness === */
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
