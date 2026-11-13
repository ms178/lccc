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
        __r[__i] = __p[__i];
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_ps(float *__p, __m256 __a)
{
    for (int __i = 0; __i < 8; __i++)
        __p[__i] = __a[__i];
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_loadu_pd(double const *__p)
{
    __m256d __r;
    for (int __i = 0; __i < 4; __i++)
        __r[__i] = __p[__i];
    return __r;
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_pd(double *__p, __m256d __a)
{
    for (int __i = 0; __i < 4; __i++)
        __p[__i] = __a[__i];
}

/* === Set === */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_setzero_si256(void)
{
    return (__m256i){  0LL, 0LL, 0LL, 0LL  };
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_setzero_ps(void)
{
    return (__m256){  0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f  };
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_setzero_pd(void)
{
    return (__m256d){  0.0, 0.0, 0.0, 0.0  };
}

/* Equal-width casts change only the lane interpretation. Native vector casts
 * preserve all 256 bits, including NaN payloads, without FP arithmetic. */

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_castps_si256(__m256 __a)
{
    return (__m256i)__a;
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_castsi256_ps(__m256i __a)
{
    return (__m256)__a;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_castps_pd(__m256 __a)
{
    return (__m256d)__a;
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_castpd_ps(__m256d __a)
{
    return (__m256)__a;
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_castpd_si256(__m256d __a)
{
    return (__m256i)__a;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_castsi256_pd(__m256i __a)
{
    return (__m256d)__a;
}

/* === Cast between 256-bit and 128-bit === */

/* Extract low 128 bits of __m256i as __m128i */
static __inline__ __m128i __attribute__((__always_inline__))
_mm256_castsi256_si128(__m256i __a)
{
    return (__m128i){  __a[0], __a[1]  };
}

/* Extract low 128 bits of __m256 as __m128 */
static __inline__ __m128 __attribute__((__always_inline__))
_mm256_castps256_ps128(__m256 __a)
{
    return (__m128){  __a[0], __a[1], __a[2], __a[3]  };
}

/* Zero-extend __m128i to __m256i (upper 128 bits undefined/zero) */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_castsi128_si256(__m128i __a)
{
    return (__m256i){  __a[0], __a[1], 0LL, 0LL  };
}

/* Extract 128-bit lane from __m256 (imm must be 0 or 1) */

/* === Float Arithmetic ===
 *
 * Each of these is one `vaddps`/`vsubps`/`vmulps` (or the `pd` spelling) and
 * is expressed as the corresponding builtin, not as an element-wise loop.
 * A loop body relies on the SLP vectorizer recognising the lane pattern: it
 * does so in a plain expression, but not through a bit-level `union`
 * (measured on the bitwise forms) and not at all at -O1, where the loop
 * became eight scalar `vaddss`.  GCC and Clang lower these to the builtin
 * unconditionally, and so does lccc now.
 */

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_add_ps(__m256 __a, __m256 __b)
{
    return (__m256)__builtin_ia32_addps256((__v8sf)(__a), (__v8sf)(__b));
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_sub_ps(__m256 __a, __m256 __b)
{
    return (__m256)__builtin_ia32_subps256((__v8sf)(__a), (__v8sf)(__b));
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_mul_ps(__m256 __a, __m256 __b)
{
    return (__m256)__builtin_ia32_mulps256((__v8sf)(__a), (__v8sf)(__b));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_add_pd(__m256d __a, __m256d __b)
{
    return (__m256d)__builtin_ia32_addpd256((__v4df)(__a), (__v4df)(__b));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_sub_pd(__m256d __a, __m256d __b)
{
    return (__m256d)__builtin_ia32_subpd256((__v4df)(__a), (__v4df)(__b));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_mul_pd(__m256d __a, __m256d __b)
{
    return (__m256d)__builtin_ia32_mulpd256((__v4df)(__a), (__v4df)(__b));
}

/* === Set (broadcast) === */

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_set1_pd(double __w)
{
    return (__m256d){  __w, __w, __w, __w  };
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_set1_ps(float __w)
{
    return (__m256){  __w, __w, __w, __w, __w, __w, __w, __w  };
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_set_pd(double __d3, double __d2, double __d1, double __d0)
{
    return (__m256d){  __d0, __d1, __d2, __d3  };
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_set_ps(float __f7, float __f6, float __f5, float __f4,
              float __f3, float __f2, float __f1, float __f0)
{
    return (__m256){  __f0, __f1, __f2, __f3, __f4, __f5, __f6, __f7  };
}

/* === Cast 256->128 for pd === */

/* Extract low 128 bits of __m256d as __m128d */
static __inline__ __m128d __attribute__((__always_inline__))
_mm256_castpd256_pd128(__m256d __a)
{
    return (__m128d){  __a[0], __a[1]  };
}

/* Zero-extend __m128d to __m256d (upper 128 bits undefined/zero) */
static __inline__ __m256d __attribute__((__always_inline__))
_mm256_castpd128_pd256(__m128d __a)
{
    return (__m256d){  __a[0], __a[1], 0.0, 0.0  };
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
    return (__m256d){
        __a[0] + __a[1],
        __b[0] + __b[1],
        __a[2] + __a[3],
        __b[2] + __b[3]
     };
}

/* Horizontal subtract: subtract adjacent pairs of doubles within each 128-bit lane */
static __inline__ __m256d __attribute__((__always_inline__))
_mm256_hsub_pd(__m256d __a, __m256d __b)
{
    return (__m256d){
        __a[0] - __a[1],
        __b[0] - __b[1],
        __a[2] - __a[3],
        __b[2] - __b[3]
     };
}

/* === Extract 128-bit lane from __m256d === */


/* === Insert 128-bit lane into __m256d === */


/* === Comparison === */



/* === Bitwise operations ===
 *
 * Bitwise AND/OR/XOR are lane-agnostic: the integer families are single
 * `vpand`/`vpor`/`vpxor` instructions and the float forms are the same
 * instruction on the same register, so each of these is a reinterpretation of
 * the integer form rather than an element-wise loop.  The previous
 * element-wise `union`/`for` bodies lowered to a per-lane GPR round trip
 * (`vmovsd`/`vmovq`/`xor`/`vmovsd` x 4 with a stack temporary) because the
 * bit-fiddle is opaque to the SLP vectorizer -- measured on
 * `tests/regression/simd_avx256_float_bitwise.c`.  `andnot` comes straight
 * from the frontend's `vpandn` builtin and must NOT be redefined here: the
 * inline definition would shadow it and reintroduce the loop.
 */

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_and_pd(__m256d __a, __m256d __b)
{
    return _mm256_castsi256_pd(
        _mm256_and_si256(_mm256_castpd_si256(__a), _mm256_castpd_si256(__b)));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_or_pd(__m256d __a, __m256d __b)
{
    return _mm256_castsi256_pd(
        _mm256_or_si256(_mm256_castpd_si256(__a), _mm256_castpd_si256(__b)));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_xor_pd(__m256d __a, __m256d __b)
{
    return _mm256_castsi256_pd(
        _mm256_xor_si256(_mm256_castpd_si256(__a), _mm256_castpd_si256(__b)));
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_and_ps(__m256 __a, __m256 __b)
{
    return _mm256_castsi256_ps(
        _mm256_and_si256(_mm256_castps_si256(__a), _mm256_castps_si256(__b)));
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_or_ps(__m256 __a, __m256 __b)
{
    return _mm256_castsi256_ps(
        _mm256_or_si256(_mm256_castps_si256(__a), _mm256_castps_si256(__b)));
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_xor_ps(__m256 __a, __m256 __b)
{
    return _mm256_castsi256_ps(
        _mm256_xor_si256(_mm256_castps_si256(__a), _mm256_castps_si256(__b)));
}

/* === Movemask === */


#endif /* _AVXINTRIN_H_INCLUDED */
