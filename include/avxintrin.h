/* CCC compiler bundled avxintrin.h - AVX intrinsics */
#ifndef _AVXINTRIN_H_INCLUDED
#define _AVXINTRIN_H_INCLUDED

#include <emmintrin.h>

/* AVX 256-bit vector types */
#define __CCC_M256I_FROM_BUILTIN(expr) (*(__m256i *)(expr))

/* Unaligned variants */
/* === Load / Store === */

/* Every 256-bit load/store below is ONE instruction at every optimization
 * level.  They used to be lane loops (`__r[i] = __p[i];`), which only became a
 * single `vmovups` when SLP ran: measured at -O0 they cost 28 instructions and
 * a 176-byte frame, i.e. the intrinsic was slower than the scalar code it was
 * supposed to replace.  The frontend builtins lower straight to the move. */
static __inline__ __m256i __attribute__((__always_inline__))
_mm256_loadu_si256(__m256i_u const *__p)
{
    return __builtin_ia32_loaddqu256(__p);
}

/* Aligned forms.  These use the same move as the unaligned ones, which is not
 * a shortcut but the encoding this backend already uses for every vector
 * dereference: `_mm_load_ps` (a plain `*(const __m128 *)p`) lowers to `movups`
 * here, so emitting `vmovaps` for the 256-bit form would make the two widths
 * disagree.  A dereference does NOT work for the 32-byte object either: the
 * backend treats it as an aggregate copy and round-trips the value through a
 * stack temporary (measured: 26 instructions for a load+store pair, versus 2
 * for the builtin).  `vmovups` from aligned storage costs exactly what
 * `vmovaps` does on every CPU that has AVX. */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_load_ps(float const *__p)
{
    return __builtin_ia32_loadups256(__p);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_store_ps(float *__p, __m256 __a)
{
    __builtin_ia32_storeups256(__p, __a);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_load_pd(double const *__p)
{
    return __builtin_ia32_loadupd256(__p);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_store_pd(double *__p, __m256d __a)
{
    __builtin_ia32_storeupd256(__p, __a);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_load_si256(__m256i const *__p)
{
    return *__p;
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_si256(__m256i_u *__p, __m256i __a)
{
    __builtin_ia32_storedqu256(__p, __a);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_store_si256(__m256i *__p, __m256i __a)
{
    *__p = __a;
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_lddqu_si256(__m256i_u const *__p)
{
    return __builtin_ia32_loaddqu256(__p);
}

/* Float load/store */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_loadu_ps(float const *__p)
{
    return __builtin_ia32_loadups256(__p);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_ps(float *__p, __m256 __a)
{
    __builtin_ia32_storeups256(__p, __a);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_loadu_pd(double const *__p)
{
    return __builtin_ia32_loadupd256(__p);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu_pd(double *__p, __m256d __a)
{
    __builtin_ia32_storeupd256(__p, __a);
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

/* === Broadcast from memory (AVX1) ===
 * Each returns every destination lane as the exact bits loaded from memory:
 * a scalar source is broadcast to all lanes and a 128-bit source is repeated
 * into both halves. Sources are read with unaligned loads, matching the Intel
 * definition, which requires no alignment for them.
 *
 * The lowering is chosen by the enabled ISA, and AVX1-only code never reaches
 * an AVX2 instruction:
 *  - AVX2: the scalar forms are _mm256_set1_*, which lower to one
 *    vbroadcastss/vbroadcastsd from memory (identical to GCC); the 128-bit
 *    forms are __builtin_ia32_vbroadcasti128 on a register value, which lowers
 *    to a register-only vinserti128 with no stack round trip.
 *  - AVX1: plain vector operations (set1 and an 8-lane shuffle), correct on
 *    any AVX CPU. They currently materialise through the stack; selecting
 *    vbroadcastss/vbroadcastsd/vbroadcastf128 under AVX1 is an open codegen
 *    item. */
#ifdef __AVX2__
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_broadcast_ss(float const *__p)
{
    return _mm256_set1_ps(*__p);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_broadcast_sd(double const *__p)
{
    return _mm256_set1_pd(*__p);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_broadcast_ps(__m128 const *__p)
{
    __m128 __x = _mm_loadu_ps((float const *)__p);
    __m256i __r = __builtin_ia32_vbroadcasti128((__m128i)__x);
    return (__m256)__r;
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_broadcast_pd(__m128d const *__p)
{
    __m128d __x = _mm_loadu_pd((double const *)__p);
    __m256i __r = __builtin_ia32_vbroadcasti128((__m128i)__x);
    return (__m256d)__r;
}
#else
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_broadcast_ss(float const *__p)
{
    return _mm256_set1_ps(*__p);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_broadcast_sd(double const *__p)
{
    return _mm256_set1_pd(*__p);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_broadcast_ps(__m128 const *__p)
{
    __m128 __x = _mm_loadu_ps((float const *)__p);
    return (__m256)__builtin_shufflevector(__x, __x, 0, 1, 2, 3, 0, 1, 2, 3);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_broadcast_pd(__m128d const *__p)
{
    __m128d __x = _mm_loadu_pd((double const *)__p);
    return (__m256d)__builtin_shufflevector(__x, __x, 0, 1, 0, 1);
}
#endif

/* === Cast 256->128 for pd === */

/* Extract low 128 bits of __m256d as __m128d */
static __inline__ __m128d __attribute__((__always_inline__))
_mm256_castpd256_pd128(__m256d __a)
{
    return (__m128d){  __a[0], __a[1]  };
}

/* Widen __m128 to __m256 (upper 128 bits undefined per Intel; the low half is
 * repeated into both halves so every lane is defined). */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_castps128_ps256(__m128 __a)
{
#ifdef __AVX2__
    __m256i __r = __builtin_ia32_vbroadcasti128((__m128i)__a);
    return (__m256)__r;
#else
    return (__m256)__builtin_shufflevector(__a, __a, 0, 1, 2, 3, 0, 1, 2, 3);
#endif
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
    return (__m256d)__builtin_ia32_andpd256((__v4df)(__a), (__v4df)(__b));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_or_pd(__m256d __a, __m256d __b)
{
    return (__m256d)__builtin_ia32_orpd256((__v4df)(__a), (__v4df)(__b));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_xor_pd(__m256d __a, __m256d __b)
{
    return (__m256d)__builtin_ia32_xorpd256((__v4df)(__a), (__v4df)(__b));
}

/* The `andnot` forms are declared here as macros over the frontend builtin.
 * They are the only operation in this family with no `__lccc_simd256_*`
 * helper, and an inline function would shadow the builtin (the first attempt
 * did exactly that and lowered to per-lane GPR work).  Without any declaration
 * the names worked only because sema happens to map them, which left the
 * public inventory reporting them missing -- see
 * engineering/isa/sse-avx-public-api.json. */
#define _mm256_andnot_ps(__a, __b) \
    (__m256)__builtin_ia32_andnps256((__v8sf)(__a), (__v8sf)(__b))

#define _mm256_andnot_pd(__a, __b) \
    (__m256d)__builtin_ia32_andnpd256((__v4df)(__a), (__v4df)(__b))

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_and_ps(__m256 __a, __m256 __b)
{
    return (__m256)__builtin_ia32_andps256((__v8sf)(__a), (__v8sf)(__b));
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_or_ps(__m256 __a, __m256 __b)
{
    return (__m256)__builtin_ia32_orps256((__v8sf)(__a), (__v8sf)(__b));
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_xor_ps(__m256 __a, __m256 __b)
{
    return (__m256)__builtin_ia32_xorps256((__v8sf)(__a), (__v8sf)(__b));
}

/* === Pure-C AVX1 completions ===
 * Each function below is a plain lane rearrangement or a scalar conversion
 * over the existing 128/256-bit builtins: no new IR is needed. The set of
 * names is the GCC 16.2 `-mavx` declaration surface that lccc did not yet
 * provide. Lane order follows the Intel definitions: `_mm256_set_*` lists the
 * highest lane first, `_mm256_setr_*` the lowest first, and `_mm256_set_m128*`
 * takes (hi, lo). Shuffle indices are constant, so every operation lowers to
 * a single register rearrangement rather than a memory round trip. */

static __inline__ float __attribute__((__always_inline__))
_mm256_cvtss_f32(__m256 __a)
{
    return _mm_cvtss_f32(_mm256_castps256_ps128(__a));
}

static __inline__ double __attribute__((__always_inline__))
_mm256_cvtsd_f64(__m256d __a)
{
    return _mm_cvtsd_f64(_mm256_castpd256_pd128(__a));
}

static __inline__ int __attribute__((__always_inline__))
_mm256_cvtsi256_si32(__m256i __a)
{
    return _mm_cvtsi128_si32(_mm256_castsi256_si128(__a));
}

/* "Undefined" values may be any value; a defined zero is the portable choice
 * and avoids reading an indeterminate register. */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_undefined_ps(void)
{
    return _mm256_setzero_ps();
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_undefined_pd(void)
{
    return _mm256_setzero_pd();
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_undefined_si256(void)
{
    return _mm256_setzero_si256();
}

static __inline__ void __attribute__((__always_inline__))
_mm256_zeroupper(void)
{
    __builtin_ia32_vzeroupper();
}

/* _mm_broadcastsd_pd (VMOVDDUP / VPERMILPD $0): duplicate the low double.
   GCC defines it as _mm_movedup_pd, the same unpacklo of a value with
   itself.  It is AVX1, so it is not behind __AVX2__. */
static __inline__ __m128d __attribute__((__always_inline__))
_mm_broadcastsd_pd(__m128d __a)
{
    return _mm_unpacklo_pd(__a, __a);
}

/* vzeroall writes every YMM register (and zeroes bits 128..255 of each).
   The asm clobber list names all sixteen registers so that a YMM value
   live across the call is spilled and reloaded rather than kept in a
   register that vzeroall destroys. */
static __inline__ void __attribute__((__always_inline__))
_mm256_zeroall(void)
{
    __asm__ volatile("vzeroall"
                     :
                     :
                     : "xmm0", "xmm1", "xmm2", "xmm3", "xmm4", "xmm5", "xmm6",
                       "xmm7", "xmm8", "xmm9", "xmm10", "xmm11", "xmm12",
                       "xmm13", "xmm14", "xmm15", "memory");
}

/* Zero-extend a 128-bit value: lanes 4..7 are the zero vector's lanes. */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_zextps128_ps256(__m128 __a)
{
    return __builtin_shufflevector(_mm256_castps128_ps256(__a),
                                   _mm256_setzero_ps(), 0, 1, 2, 3, 8, 9, 10, 11);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_zextpd128_pd256(__m128d __a)
{
    return __builtin_shufflevector(_mm256_castpd128_pd256(__a),
                                   _mm256_setzero_pd(), 0, 1, 6, 7);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_set_m128(__m128 __hi, __m128 __lo)
{
    return __builtin_shufflevector(_mm256_castps128_ps256(__lo),
                                   _mm256_castps128_ps256(__hi),
                                   0, 1, 2, 3, 8, 9, 10, 11);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_set_m128d(__m128d __hi, __m128d __lo)
{
    return __builtin_shufflevector(_mm256_castpd128_pd256(__lo),
                                   _mm256_castpd128_pd256(__hi), 0, 1, 4, 5);
}

/* __m256i has four 64-bit lanes: lo is lanes 0-1, hi is lanes 2-3. */
static __inline__ __m256i __attribute__((__always_inline__))
__lccc_set_m128i_avx(__m128i __hi, __m128i __lo)
{
    return __builtin_shufflevector(_mm256_castsi128_si256(__lo),
                                   _mm256_castsi128_si256(__hi), 0, 1, 4, 5);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_setr_m128(__m128 __lo, __m128 __hi)
{
    return _mm256_set_m128(__hi, __lo);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_setr_m128d(__m128d __lo, __m128d __hi)
{
    return _mm256_set_m128d(__hi, __lo);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_setr_m128i(__m128i __lo, __m128i __hi)
{
    return __lccc_set_m128i_avx(__hi, __lo);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_setr_ps(float __f0, float __f1, float __f2, float __f3,
               float __f4, float __f5, float __f6, float __f7)
{
    return _mm256_set_ps(__f7, __f6, __f5, __f4, __f3, __f2, __f1, __f0);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_setr_pd(double __d0, double __d1, double __d2, double __d3)
{
    return _mm256_set_pd(__d3, __d2, __d1, __d0);
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_setr_epi64x(long long __e0, long long __e1, long long __e2, long long __e3)
{
    return (__m256i){__e0, __e1, __e2, __e3};
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_set_epi16(short __e15, short __e14, short __e13, short __e12,
                 short __e11, short __e10, short __e9, short __e8,
                 short __e7, short __e6, short __e5, short __e4,
                 short __e3, short __e2, short __e1, short __e0)
{
    return __lccc_set_m128i_avx(_mm_set_epi16(__e15, __e14, __e13, __e12,
                                              __e11, __e10, __e9, __e8),
                                _mm_set_epi16(__e7, __e6, __e5, __e4,
                                              __e3, __e2, __e1, __e0));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_setr_epi16(short __e0, short __e1, short __e2, short __e3,
                  short __e4, short __e5, short __e6, short __e7,
                  short __e8, short __e9, short __e10, short __e11,
                  short __e12, short __e13, short __e14, short __e15)
{
    return _mm256_set_epi16(__e15, __e14, __e13, __e12, __e11, __e10, __e9, __e8,
                            __e7, __e6, __e5, __e4, __e3, __e2, __e1, __e0);
}

/* loadu2/storeu2: the first pointer addresses the high half, the second the
 * low half, exactly as Intel defines them. */
static __inline__ __m256 __attribute__((__always_inline__))
_mm256_loadu2_m128(float const *__hi, float const *__lo)
{
    return _mm256_set_m128(_mm_loadu_ps(__hi), _mm_loadu_ps(__lo));
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_loadu2_m128d(double const *__hi, double const *__lo)
{
    return _mm256_set_m128d(_mm_loadu_pd(__hi), _mm_loadu_pd(__lo));
}

static __inline__ __m256i __attribute__((__always_inline__))
_mm256_loadu2_m128i(__m128i const *__hi, __m128i const *__lo)
{
    return __lccc_set_m128i_avx(_mm_loadu_si128(__hi), _mm_loadu_si128(__lo));
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu2_m128(float *__hi, float *__lo, __m256 __a)
{
    _mm_storeu_ps(__lo, _mm256_castps256_ps128(__a));
    _mm_storeu_ps(__hi, __builtin_shufflevector(__a, __a, 4, 5, 6, 7));
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu2_m128d(double *__hi, double *__lo, __m256d __a)
{
    _mm_storeu_pd(__lo, _mm256_castpd256_pd128(__a));
    _mm_storeu_pd(__hi, __builtin_shufflevector(__a, __a, 2, 3));
}

static __inline__ void __attribute__((__always_inline__))
_mm256_storeu2_m128i(__m128i *__hi, __m128i *__lo, __m256i __a)
{
    _mm_storeu_si128(__lo, _mm256_castsi256_si128(__a));
    _mm_storeu_si128(__hi, (__m128i)__builtin_shufflevector(__a, __a, 2, 3));
}

/* Duplicate even or odd lanes. */
static __inline__ __m256d __attribute__((__always_inline__))
_mm256_movedup_pd(__m256d __a)
{
    return __builtin_shufflevector(__a, __a, 0, 0, 2, 2);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_movehdup_ps(__m256 __a)
{
    return __builtin_shufflevector(__a, __a, 1, 1, 3, 3, 5, 5, 7, 7);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_moveldup_ps(__m256 __a)
{
    return __builtin_shufflevector(__a, __a, 0, 0, 2, 2, 4, 4, 6, 6);
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_broadcast_ss(float const *__p)
{
    return _mm_set1_ps(*__p);
}

/* In-lane permutes with an immediate selector. The selector must be a
 * constant, so these are macros: the shuffle indices are then constant
 * expressions as `__builtin_shufflevector` requires. */
#define _mm_permute_ps(__a, __imm)                                              \
    (__LCCC_IMMEDIATE(__imm, 255, "the last argument must be an 8-bit immediate"), \
     __builtin_shufflevector((__a), (__a), ((__imm) >> 0) & 3, ((__imm) >> 2) & 3, \
                             ((__imm) >> 4) & 3, ((__imm) >> 6) & 3))
#define _mm_permute_pd(__a, __imm)                                              \
    (__LCCC_IMMEDIATE(__imm, 255, "the last argument must be an 8-bit immediate"), \
     __builtin_shufflevector((__a), (__a), ((__imm) >> 0) & 1, ((__imm) >> 1) & 1))

#define _mm256_ceil_ps(__a) _mm256_round_ps((__a), _MM_FROUND_CEIL)
#define _mm256_floor_ps(__a) _mm256_round_ps((__a), _MM_FROUND_FLOOR)
#define _mm256_ceil_pd(__a) _mm256_round_pd((__a), _MM_FROUND_CEIL)
#define _mm256_floor_pd(__a) _mm256_round_pd((__a), _MM_FROUND_FLOOR)

#define _mm256_permute2f128_si256(__a, __b, __imm)                              \
    _mm256_castps_si256(_mm256_permute2f128_ps(_mm256_castsi256_ps(__a),        \
                                               _mm256_castsi256_ps(__b), (__imm)))

/* === 16-bit element extract and insert (GCC-compatible, under -mavx) ===
 * Element N (0..15) lives in lane-128 half (N >> 3) at word (N & 7). Both are
 * macros so the half selector and the word index stay compile-time constants,
 * which the lane-128 gate and the lane-128 lowering require. An N past the
 * vector is refused, as GCC refuses it (there by the half it would select). */
#define _mm256_extract_epi16(__X, __N)                                      \
    (__LCCC_IMMEDIATE(__N, 15, "selector must be an integer constant in the range [0, 15]"), \
     _mm_extract_epi16(_mm256_extractf128_si256((__X), ((__N) >> 3) & 1), (__N) & 7))

#define _mm256_insert_epi16(__X, __D, __N)                                  \
    (__LCCC_IMMEDIATE(__N, 15, "selector must be an integer constant in the range [0, 15]"), \
     _mm256_insertf128_si256((__X),                                         \
        _mm_insert_epi16(_mm256_extractf128_si256((__X), ((__N) >> 3) & 1), \
                         (__D), (__N) & 7),                                 \
        ((__N) >> 3) & 1))

/* === Movemask === */


/* === Non-temporal stores and sign-bit-masked moves (AVX1) ===
 * The streaming stores (VMOVNT*) require a 32-byte aligned destination. The
 * masked moves (VMASKMOV*) use only the sign bit of each lane: a masked-off
 * lane is neither read nor written, so it cannot fault. */

static __inline__ void __attribute__((__always_inline__))
_mm256_stream_ps(float *__p, __m256 __a)
{
    __builtin_ia32_movntps256(__p, (__v8sf)__a);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_stream_pd(double *__p, __m256d __a)
{
    __builtin_ia32_movntpd256(__p, (__v4df)__a);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_stream_si256(__m256i *__p, __m256i __a)
{
    __builtin_ia32_movntdq256((__v4di *)__p, (__v4di)__a);
}

static __inline__ __m256 __attribute__((__always_inline__))
_mm256_maskload_ps(float const *__p, __m256i __m)
{
    return (__m256)__builtin_ia32_maskloadps256((const __v8sf *)__p, (__v8si)__m);
}

static __inline__ __m256d __attribute__((__always_inline__))
_mm256_maskload_pd(double const *__p, __m256i __m)
{
    return (__m256d)__builtin_ia32_maskloadpd256((const __v4df *)__p, (__v4di)__m);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_maskstore_ps(float *__p, __m256i __m, __m256 __a)
{
    __builtin_ia32_maskstoreps256((__v8sf *)__p, (__v8si)__m, (__v8sf)__a);
}

static __inline__ void __attribute__((__always_inline__))
_mm256_maskstore_pd(double *__p, __m256i __m, __m256d __a)
{
    __builtin_ia32_maskstorepd256((__v4df *)__p, (__v4di)__m, (__v4df)__a);
}

static __inline__ __m128 __attribute__((__always_inline__))
_mm_maskload_ps(float const *__p, __m128i __m)
{
    return (__m128)__builtin_ia32_maskloadps((const __v4sf *)__p, (__v4si)__m);
}

static __inline__ __m128d __attribute__((__always_inline__))
_mm_maskload_pd(double const *__p, __m128i __m)
{
    return (__m128d)__builtin_ia32_maskloadpd((const __v2df *)__p, (__v2di)__m);
}

static __inline__ void __attribute__((__always_inline__))
_mm_maskstore_ps(float *__p, __m128i __m, __m128 __a)
{
    __builtin_ia32_maskstoreps((__v4sf *)__p, (__v4si)__m, (__v4sf)__a);
}

static __inline__ void __attribute__((__always_inline__))
_mm_maskstore_pd(double *__p, __m128i __m, __m128d __a)
{
    __builtin_ia32_maskstorepd((__v2df *)__p, (__v2di)__m, (__v2df)__a);
}

#endif /* _AVXINTRIN_H_INCLUDED */
