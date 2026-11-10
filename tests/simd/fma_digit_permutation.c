/* Differential correctness for the FMA3 digit permutations (`vfmadd{132,213,
 * 231}{ps,pd}` at 128 and 256 bits).
 *
 * The defect (engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md, SA-11):
 * all six spellings shared ONE operand placement (`rm=b, vvvv=c, dst=a`).
 * Intel SDM maps the digit triple onto the roles as
 *
 *     132:  dst = dst  * src3 + src2
 *     213:  dst = src2 * dst  + src3
 *     231:  dst = src2 * src3 + dst
 *
 * so with a uniform reading of `(a, b, c)` those are three DIFFERENT
 * expressions.  Measured on this host with a=3, b=5, c=7:
 *
 *     132 -> 22.0  == a*b + c   correct
 *     213 -> 26.0  == c*a + b   WRONG
 *     231 -> 38.0  == b*c + a   WRONG
 *
 * Only 132 was ever right.  213 is not fixable by swapping its two
 * multiplicand slots (both are multiplicands there) -- the ADDEND role has to
 * move -- so each spelling needs its own assignment.
 *
 * The distinction matters because -O2 will happily fold a constant
 * `a*b + c` at compile time, which is why every kernel below reads its
 * operands from `volatile` storage and folds the result into a checksum the
 * compiler cannot predict.
 *
 * These spellings are reachable only through the raw `__lccc_simd*` builtins:
 * the public `_mm_fmadd_ps` / `_mm256_fmadd_ps` macros always use 132, so
 * this file declares the raw prototypes itself (GCC and Clang do not have
 * them, so the whole body is guarded by `FMA_DIGIT_PROBE` and the file still compiles
 * -- and prints nothing -- under a reference compiler; the gate compares LCCC
 * against `a*b + c` computed by the reference, not row-for-row).
 *
 * Build:  <cc> -O2 -mfma -mavx2 this.c -o /tmp/fma
 * Run:    /tmp/fma
 *
 * No UB: unsigned/float arithmetic only, every value read from an initialised
 * object, no aliasing violation (all vector traffic goes through the vector
 * types themselves), no out-of-bounds access.  The comparisons are on exact
 * values representable in binary32 (products and sums of small integers), so
 * FMA's single rounding equals the two-step result and the test does not
 * depend on contraction behaviour.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* `FMA_DIGIT_PROBE` selects the mode that calls LCCC's raw `__lccc_simd*`
 * builtins.  It is a property of the COMPILER being exercised, which this
 * file cannot detect for itself: there is no `__CCC__` predefined macro (it
 * was previously assumed here and exists nowhere in the compiler, so the whole
 * exercised body was silently skipped under real lccc and the gate compared
 * zero rows).  The driver
 * (`tests/regression/check_fma_digit_permutation.sh`) passes the define when
 * `--lccc` names a compiler that provides those builtins, and omits it for a
 * reference compiler, which then gets just the scalar `ref_*` rows. */
#if defined(FMA_DIGIT_PROBE)
/* Only `<immintrin.h>` is included, never `<lcccsimd.h>`: the probe has to
 * build under the pre-fix STAND-IN as well, and that runs the repository's
 * headers through a reference compiler, whose builtins differ from lccc's.
 * `<immintrin.h>` gives every compiler in play the `__m128`/`__m256` types and
 * the load/store intrinsics; the two `__CCC_*_FROM_BUILTIN` conversion macros
 * for the raw builtins are then defined below unless a header already did. */
#include <immintrin.h>
#ifndef __CCC_M256_FROM_BUILTIN
/* The shipped contract: a `void *`-returning `__lccc_simd256_*` builtin
 * yields the ADDRESS of the result vector, so it is dereferenced, not raced
 * through a pointer-sized `memcpy` (which reads the address's low bytes). */
#define __CCC_M256_FROM_BUILTIN(e)  (*(__m256 *)(e))
#define __CCC_M256D_FROM_BUILTIN(e) (*(__m256d *)(e))
#endif

/* Raw builtin prototypes the headers do not ship (they exist only so the
 * lowering table can be exercised directly).  The leading `long` is the
 * dummy immediate-class argument every `__lccc_simd*` builtin takes. */
__m128 __lccc_simd128_ps_fmadd132ps128(long, __m128, __m128, __m128);
__m128 __lccc_simd128_ps_fmadd213ps128(long, __m128, __m128, __m128);
__m128 __lccc_simd128_ps_fmadd231ps128(long, __m128, __m128, __m128);
__m128d __lccc_simd128_ps_fmadd132pd128(long, __m128d, __m128d, __m128d);
__m128d __lccc_simd128_ps_fmadd213pd128(long, __m128d, __m128d, __m128d);
__m128d __lccc_simd128_ps_fmadd231pd128(long, __m128d, __m128d, __m128d);
void *__lccc_simd256_ps_vfmadd132ps256(long, __m256, __m256, __m256);
void *__lccc_simd256_ps_vfmadd213ps256(long, __m256, __m256, __m256);
void *__lccc_simd256_ps_vfmadd231ps256(long, __m256, __m256, __m256);
void *__lccc_simd256_ps_vfmadd132pd256(long, __m256d, __m256d, __m256d);
void *__lccc_simd256_ps_vfmadd213pd256(long, __m256d, __m256d, __m256d);
void *__lccc_simd256_ps_vfmadd231pd256(long, __m256d, __m256d, __m256d);
#endif

/* Distinct, exactly-representable payloads so a wrong permutation produces a
 * DIFFERENT value rather than a coincidentally equal one.  `volatile` forces a
 * real load, defeating compile-time folding. */
/* Full width: the 256-bit blocks load a whole __m256/__m256d, so a 4-wide
 * table leaves lanes 4..7 zero and lets two lanes agree by accident.  Every
 * lane below gives a DIFFERENT a*b+c, and its 213/231 misreadings differ from
 * it too, so a wrong permutation cannot hide behind an equal value.  The
 * doubles use their own magnitudes so no ps lane equals a pd lane. */
static const float FA[8] = {3.0f, 5.0f, 7.0f, 11.0f, 13.0f, 17.0f, 19.0f, 23.0f};
static const float FB[8] = {5.0f, 7.0f, 11.0f, 3.0f, 17.0f, 19.0f, 23.0f, 13.0f};
static const float FC[8] = {7.0f, 11.0f, 3.0f, 5.0f, 19.0f, 23.0f, 13.0f, 17.0f};
static const double DA[4] = {2.0, 6.0, 9.0, 14.0};
static const double DB[4] = {4.0, 8.0, 13.0, 5.0};
static const double DC[4] = {6.0, 10.0, 2.0, 7.0};

/* 8, not 4: the 256-bit ps lanes consume a full __m256 (32 bytes), so a
 * 4-element array would be read 16 bytes past its end. */
static volatile float fa_v[8], fb_v[8], fc_v[8];
/* 4, not 2: the 256-bit pd lanes consume a full __m256d (32 bytes), so a
 * 2-element array would be read 16 bytes past its end. */
static volatile double da_v[4], db_v[4], dc_v[4];

/* Fold four floats into a checksum without letting the optimiser anticipate
 * the value: the bytes go through `unsigned char`, which may alias anything. */
static uint64_t mix(uint64_t h, const void *p, size_t n) {
    const unsigned char *b = (const unsigned char *)p;
    for (size_t i = 0; i < n; i++) {
        h ^= (uint64_t)b[i];
        h *= 1099511628211ULL;
    }
    return h;
}

int main(void) {
    memcpy((void *)fa_v, FA, sizeof FA);
    memcpy((void *)fb_v, FB, sizeof FB);
    memcpy((void *)fc_v, FC, sizeof FC);
    memcpy((void *)da_v, DA, sizeof DA);
    memcpy((void *)db_v, DB, sizeof DB);
    memcpy((void *)dc_v, DC, sizeof DC);

    uint64_t h = 1469598103934665603ULL;

    /* The reference answer, computed scalar-wise as a*b + c.  Emitted as a row
     * of its own so the file is meaningful under a reference compiler too:
     * `ref_*` rows are what LCCC's `*_132/213/231` rows must reproduce. */
    /* Sized for the widest block below (the 256-bit ones fill 8/4 lanes). */
    float want_f[8];
    for (int i = 0; i < 4; i++) {
        want_f[i] = fa_v[i] * fb_v[i] + fc_v[i];
        printf("ref_ps128 lane%d %.1f\n", i, (double)want_f[i]);
    }
    double want_d[4];
    for (int i = 0; i < 2; i++) {
        want_d[i] = da_v[i] * db_v[i] + dc_v[i];
        printf("ref_pd128 lane%d %.1f\n", i, want_d[i]);
    }
    h = mix(h, want_f, sizeof want_f);
    h = mix(h, want_d, sizeof want_d);

#if defined(FMA_DIGIT_PROBE)
    {
        __m128 a = _mm_loadu_ps((const float *)fa_v);
        __m128 b = _mm_loadu_ps((const float *)fb_v);
        __m128 c = _mm_loadu_ps((const float *)fc_v);
        float got132[4], got213[4], got231[4];
        _mm_storeu_ps(got132, __lccc_simd128_ps_fmadd132ps128(0, a, b, c));
        _mm_storeu_ps(got213, __lccc_simd128_ps_fmadd213ps128(0, a, b, c));
        _mm_storeu_ps(got231, __lccc_simd128_ps_fmadd231ps128(0, a, b, c));
        h = mix(h, got132, sizeof got132);
        h = mix(h, got213, sizeof got213);
        h = mix(h, got231, sizeof got231);
        /* Report per spelling so a failure names the permutation. */
        for (int i = 0; i < 4; i++) {
            printf("ps128_132 lane%d %.1f want %.1f\n", i, (double)got132[i],
                   (double)want_f[i]);
            printf("ps128_213 lane%d %.1f want %.1f\n", i, (double)got213[i],
                   (double)want_f[i]);
            printf("ps128_231 lane%d %.1f want %.1f\n", i, (double)got231[i],
                   (double)want_f[i]);
        }
    }
    {
        __m128d a = _mm_loadu_pd((const double *)da_v);
        __m128d b = _mm_loadu_pd((const double *)db_v);
        __m128d c = _mm_loadu_pd((const double *)dc_v);
        double got132[2], got213[2], got231[2];
        _mm_storeu_pd(got132, __lccc_simd128_ps_fmadd132pd128(0, a, b, c));
        _mm_storeu_pd(got213, __lccc_simd128_ps_fmadd213pd128(0, a, b, c));
        _mm_storeu_pd(got231, __lccc_simd128_ps_fmadd231pd128(0, a, b, c));
        h = mix(h, got132, sizeof got132);
        h = mix(h, got213, sizeof got213);
        h = mix(h, got231, sizeof got231);
        for (int i = 0; i < 2; i++) {
            printf("pd128_132 lane%d %.1f want %.1f\n", i, got132[i], want_d[i]);
            printf("pd128_213 lane%d %.1f want %.1f\n", i, got213[i], want_d[i]);
            printf("pd128_231 lane%d %.1f want %.1f\n", i, got231[i], want_d[i]);
        }
    }
    {
        /* The 256-bit forms return `void *` (the header's generic carrier), so
         * the result reaches a vector type through a `memcpy` -- exact, free,
         * and never a strict-aliasing violation. */
        __m256 a = _mm256_loadu_ps((const float *)fa_v);
        __m256 b = _mm256_loadu_ps((const float *)fb_v);
        __m256 c = _mm256_loadu_ps((const float *)fc_v);
        float got[8];
        for (int lane = 0; lane < 8; lane++) {
            want_f[lane] = fa_v[lane] * fb_v[lane] + fc_v[lane];
            printf("ref_ps256 lane%d %.1f\n", lane, (double)want_f[lane]);
        }
        /* The header's own contract for a `void *`-returning builtin:
         * the pointer is the ADDRESS of the result vector, so it must be
         * dereferenced.  Reading the pointer value instead yields the address,
         * not the lanes. */
        { __m256 r = __CCC_M256_FROM_BUILTIN(__lccc_simd256_ps_vfmadd132ps256(0, a, b, c));
          memcpy(got, &r, sizeof r); }
        h = mix(h, got, sizeof got);
        for (int i = 0; i < 8; i++) {
            printf("ps256_132 lane%d %.1f want %.1f\n", i, (double)got[i],
                   (double)want_f[i]);
        }
        /* The header's own contract for a `void *`-returning builtin:
         * the pointer is the ADDRESS of the result vector, so it must be
         * dereferenced.  Reading the pointer value instead yields the address,
         * not the lanes. */
        { __m256 r = __CCC_M256_FROM_BUILTIN(__lccc_simd256_ps_vfmadd213ps256(0, a, b, c));
          memcpy(got, &r, sizeof r); }
        h = mix(h, got, sizeof got);
        for (int i = 0; i < 8; i++) {
            printf("ps256_213 lane%d %.1f want %.1f\n", i, (double)got[i],
                   (double)want_f[i]);
        }
        /* The header's own contract for a `void *`-returning builtin:
         * the pointer is the ADDRESS of the result vector, so it must be
         * dereferenced.  Reading the pointer value instead yields the address,
         * not the lanes. */
        { __m256 r = __CCC_M256_FROM_BUILTIN(__lccc_simd256_ps_vfmadd231ps256(0, a, b, c));
          memcpy(got, &r, sizeof r); }
        h = mix(h, got, sizeof got);
        for (int i = 0; i < 8; i++) {
            printf("ps256_231 lane%d %.1f want %.1f\n", i, (double)got[i],
                   (double)want_f[i]);
        }
    }
    {
        __m256d a = _mm256_loadu_pd((const double *)da_v);
        __m256d b = _mm256_loadu_pd((const double *)db_v);
        __m256d c = _mm256_loadu_pd((const double *)dc_v);
        double got[4];
        for (int lane = 0; lane < 4; lane++) {
            want_d[lane] = da_v[lane] * db_v[lane] + dc_v[lane];
            printf("ref_pd256 lane%d %.1f\n", lane, want_d[lane]);
        }
        /* The header's own contract for a `void *`-returning builtin:
         * the pointer is the ADDRESS of the result vector, so it must be
         * dereferenced.  Reading the pointer value instead yields the address,
         * not the lanes. */
        { __m256d r = __CCC_M256D_FROM_BUILTIN(__lccc_simd256_ps_vfmadd132pd256(0, a, b, c));
          memcpy(got, &r, sizeof r); }
        h = mix(h, got, sizeof got);
        for (int i = 0; i < 4; i++) {
            printf("pd256_132 lane%d %.1f want %.1f\n", i, got[i], want_d[i]);
        }
        /* The header's own contract for a `void *`-returning builtin:
         * the pointer is the ADDRESS of the result vector, so it must be
         * dereferenced.  Reading the pointer value instead yields the address,
         * not the lanes. */
        { __m256d r = __CCC_M256D_FROM_BUILTIN(__lccc_simd256_ps_vfmadd213pd256(0, a, b, c));
          memcpy(got, &r, sizeof r); }
        h = mix(h, got, sizeof got);
        for (int i = 0; i < 4; i++) {
            printf("pd256_213 lane%d %.1f want %.1f\n", i, got[i], want_d[i]);
        }
        /* The header's own contract for a `void *`-returning builtin:
         * the pointer is the ADDRESS of the result vector, so it must be
         * dereferenced.  Reading the pointer value instead yields the address,
         * not the lanes. */
        { __m256d r = __CCC_M256D_FROM_BUILTIN(__lccc_simd256_ps_vfmadd231pd256(0, a, b, c));
          memcpy(got, &r, sizeof r); }
        h = mix(h, got, sizeof got);
        for (int i = 0; i < 4; i++) {
            printf("pd256_231 lane%d %.1f want %.1f\n", i, got[i], want_d[i]);
        }
    }
#endif
    printf("checksum %016llx\n", (unsigned long long)h);
    return 0;
}
