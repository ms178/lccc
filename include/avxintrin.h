/* CCC compiler bundled avxintrin.h - AVX intrinsics */
#ifndef _AVXINTRIN_H_INCLUDED
#define _AVXINTRIN_H_INCLUDED

#include <emmintrin.h>

/* AVX 256-bit vector types */
#define __CCC_M256I_FROM_BUILTIN(expr) (*(__m256i *)(expr))

/* Unaligned variants */
/* === Load / Store === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_loadu_si256(__m256i_u const *__p)
{
    __m256i __r;
    __builtin_memcpy(&__r, __p, sizeof(__r));
    return __r;
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_load_si256(__m256i const *__p)
{
    return *__p;
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_si256(__m256i_u *__p, __m256i __a)
{
    __builtin_memcpy(__p, &__a, sizeof(__a));
}

static __inline__ void __attribute__((__always_inline__))
_mm256_store_si256(__m256i *__p, __m256i __a)
{
    *__p = __a;
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_lddqu_si256(__m256i_u const *__p)
{
    __m256i __r;
    __builtin_memcpy(&__r, __p, sizeof(__r));
    return __r;
}

/* Float load/store */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_loadu_ps(float const *__p)
{
    __m256 __r;
    for (int __i = 0; __i < 8; __i++)
        __r.__val[__i] = __p[__i];
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_ps(float *__p, __m256 __a)
{
    for (int __i = 0; __i < 8; __i++)
        __p[__i] = __a.__val[__i];
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_loadu_pd(double const *__p)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++)
        __r.__val[__i] = __p[__i];
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_pd(double *__p, __m256d __a)
{
    for (int __i = 0; __i < 4; __i++)
        __p[__i] = __a.__val[__i];
}

/* === Set === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_setzero_si256(void)
{
    return (__m256i){ { 0LL, 0LL, 0LL, 0LL } };
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_setzero_ps(void)
{
    return (__m256){ { 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f } };
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_setzero_pd(void)
{
    return (__m256d){ { 0.0, 0.0, 0.0, 0.0 } };
}

/* === Cast between 256-bit and 128-bit === */

/* Extract low 128 bits of __m256i as __m128i */
static __inline__ __m128i __attribute__((__always_inline__))
_mm256_castsi256_si128(__m256i __a)
{
    return (__m128i){ { __a.__val[0], __a.__val[1] } };
}

/* Extract low 128 bits of __m256 as __m128 */
static __inline__ __m128 __attribute__((__always_inline__))
_mm256_castps256_ps128(__m256 __a)
{
    return (__m128){ { __a.__val[0], __a.__val[1], __a.__val[2], __a.__val[3] } };
}

/* Zero-extend __m128i to __m256i (upper 128 bits undefined/zero) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_castsi128_si256(__m128i __a)
{
    return (__m256i){ { __a.__val[0], __a.__val[1], 0LL, 0LL } };
}

/* Extract 128-bit lane from __m256 (imm must be 0 or 1) */

/* === Float Arithmetic === */

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_add_ps(__m256 __a, __m256 __b)
{
    __m256 __r;
    for (int __i = 0; __i < 8; __i++)
        __r.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __r;
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_sub_ps(__m256 __a, __m256 __b)
{
    __m256 __r;
    for (int __i = 0; __i < 8; __i++)
        __r.__val[__i] = __a.__val[__i] - __b.__val[__i];
    return __r;
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_mul_ps(__m256 __a, __m256 __b)
{
    __m256 __r;
    for (int __i = 0; __i < 8; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i];
    return __r;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_add_pd(__m256d __a, __m256d __b)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++)
        __r.__val[__i] = __a.__val[__i] + __b.__val[__i];
    return __r;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_sub_pd(__m256d __a, __m256d __b)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++)
        __r.__val[__i] = __a.__val[__i] - __b.__val[__i];
    return __r;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_mul_pd(__m256d __a, __m256d __b)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++)
        __r.__val[__i] = __a.__val[__i] * __b.__val[__i];
    return __r;
}

/* === Double Arithmetic (continued) === */


/* === Set (broadcast) === */

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_set1_pd(double __w)
{
    return (__m256d){ { __w, __w, __w, __w } };
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_set1_ps(float __w)
{
    return (__m256){ { __w, __w, __w, __w, __w, __w, __w, __w } };
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_set_pd(double __d3, double __d2, double __d1, double __d0)
{
    return (__m256d){ { __d0, __d1, __d2, __d3 } };
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_set_ps(float __f7, float __f6, float __f5, float __f4,
              float __f3, float __f2, float __f1, float __f0)
{
    return (__m256){ { __f0, __f1, __f2, __f3, __f4, __f5, __f6, __f7 } };
}

/* === Cast 256->128 for pd === */

/* Extract low 128 bits of __m256d as __m128d */
static __inline__ __m128d __attribute__((__always_inline__))
_mm256_castpd256_pd128(__m256d __a)
{
    return (__m128d){ { __a.__val[0], __a.__val[1] } };
}

/* Zero-extend __m128d to __m256d (upper 128 bits undefined/zero) */
static __inline__ __m256d __attribute__((__always_inline__))
_mm256_castpd128_pd256(__m128d __a)
{
    return (__m256d){ { __a.__val[0], __a.__val[1], 0.0, 0.0 } };
}

/* === Shuffle / Permute (double) === */

/* Unpack and interleave low doubles from each 128-bit lane */

/* Unpack and interleave high doubles from each 128-bit lane */

/* Shuffle doubles within each 128-bit lane based on imm8 control */

/* Permute 128-bit lanes from two 256-bit sources */

/* === Horizontal operations === */

/* Horizontal add: add adjacent pairs of doubles within each 128-bit lane */
static __inline__ __m256d __attribute__((__always_inline__))
_mm256_hadd_pd(__m256d __a, __m256d __b)
{
    return (__m256d){ {
        __a.__val[0] + __a.__val[1],
        __b.__val[0] + __b.__val[1],
        __a.__val[2] + __a.__val[3],
        __b.__val[2] + __b.__val[3]
    } };
}

/* Horizontal subtract: subtract adjacent pairs of doubles within each 128-bit lane */
static __inline__ __m256d __attribute__((__always_inline__))
_mm256_hsub_pd(__m256d __a, __m256d __b)
{
    return (__m256d){ {
        __a.__val[0] - __a.__val[1],
        __b.__val[0] - __b.__val[1],
        __a.__val[2] - __a.__val[3],
        __b.__val[2] - __b.__val[3]
    } };
}

/* === Extract 128-bit lane from __m256d === */


/* === Insert 128-bit lane into __m256d === */


/* === Comparison === */



/* === Bitwise operations (pd) === */

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_and_pd(__m256d __a, __m256d __b)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++) {
        union { double d; long long ll; } __ua, __ub, __ur;
        __ua.d = __a.__val[__i]; __ub.d = __b.__val[__i];
        __ur.ll = __ua.ll & __ub.ll;
        __r.__val[__i] = __ur.d;
    }
    return __r;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_or_pd(__m256d __a, __m256d __b)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++) {
        union { double d; long long ll; } __ua, __ub, __ur;
        __ua.d = __a.__val[__i]; __ub.d = __b.__val[__i];
        __ur.ll = __ua.ll | __ub.ll;
        __r.__val[__i] = __ur.d;
    }
    return __r;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_xor_pd(__m256d __a, __m256d __b)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++) {
        union { double d; long long ll; } __ua, __ub, __ur;
        __ua.d = __a.__val[__i]; __ub.d = __b.__val[__i];
        __ur.ll = __ua.ll ^ __ub.ll;
        __r.__val[__i] = __ur.d;
    }
    return __r;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_andnot_pd(__m256d __a, __m256d __b)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++) {
        union { double d; long long ll; } __ua, __ub, __ur;
        __ua.d = __a.__val[__i]; __ub.d = __b.__val[__i];
        __ur.ll = (~__ua.ll) & __ub.ll;
        __r.__val[__i] = __ur.d;
    }
    return __r;
}

/* === Movemask === */


#endif /* _AVXINTRIN_H_INCLUDED */
