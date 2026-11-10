/* Emulated `__lccc_simd*` FMA builtins carrying the PRE-FIX (SA-11) semantics.
 *
 * `tests/regression/fma_prefix_standin.sh` `-include`s this header on top of a
 * reference compiler so `--expect-defect` has something real to be red about.
 *
 * Why emulation is necessary: the probe's exercising body calls LCCC's raw
 * `__lccc_simd*` builtins, which no other compiler declares.  Before this
 * header existed the stand-in only `-D`-renamed the 213/231 prototypes onto
 * 132, so with a reference compiler the probe failed to compile and the gate
 * had no way to express "the defect is present" -- it merely failed to build.
 * A stand-in that cannot compile the fixture proves nothing about the fixture.
 *
 * What the defect WAS: one shared operand placement (`rm = b, vvvv = c,
 * dst = a`, the 132 placement) used for all three digit spellings.  Intel SDM:
 *
 *   132:  dst = dst  * src3 + src2      -> with that placement: a*b + c   right
 *   213:  dst = src2 * dst  + src3      -> with that placement: c*a + b   WRONG
 *   231:  dst = src2 * src3 + dst       -> with that placement: b*c + a   WRONG
 *
 * So the emulation below applies the SINGLE 132 assignment to every spelling
 * and the `_213`/`_231` rows disagree with the file's own scalar `ref_*` rows.
 * Each spelling still emits its own mnemonic-shaped work, so a test that
 * compared mnemonics alone would not catch this -- only the VALUES differ,
 * which is exactly the property the gate has to be sensitive to.
 *
 * The `void *` return is the header's own contract (`__CCC_M256_FROM_BUILTIN`
 * dereferences it), so each emulation returns the address of a live vector.
 */
#ifndef FMA_PREFIX_STANDIN_H
#define FMA_PREFIX_STANDIN_H

#include <immintrin.h>
#include <string.h>

/* One scratch vector per width; the caller copies out immediately. */
static __m128  fma_standin_r128;
static __m128d fma_standin_r128d;
static __m256  fma_standin_r256;
static __m256d fma_standin_r256d;
/* 512-bit scratch.  `__m512`/`__m512d` only exist under -mavx512f, which the
 * gate passes only when the host can execute the result, so the 512-bit
 * emulation is guarded on the same macro the compiler defines for it. */
#if defined(__AVX512F__)
static __m512  fma_standin_r512;
static __m512d fma_standin_r512d;
#endif

/* The shared PRE-FIX placement, spelled out per width: result lane =
 * a[lane] * b[lane] + c[lane] would be CORRECT, so to reproduce the defect the
 * emulation instead applies the assignment the buggy emitter used -- operands
 * are taken in the roles `rm = b, vvvv = c, dst = a` under each spelling's own
 * pseudocode.  Writing it as the pseudocode keeps it honest: the 132 row is the
 * correct one, and 213/231 are the two that were wrong. */

#define FMA_STANDIN_PS128(digits, a, b, c, out)                                  \
    do {                                                                        \
        __m128 s2 = (c), s3 = (b), d = (a); /* vvvv, rm, dst */                  \
        if ((digits) == 132) {                                                   \
            out = d * s3 + s2;   /* DEST*SRC3 + SRC2 */                          \
        } else if ((digits) == 213) {                                            \
            out = s2 * d + s3;   /* SRC2*DEST + SRC3 */                          \
        } else {                                                                 \
            out = s2 * s3 + d;   /* SRC2*SRC3 + DEST */                          \
        }                                                                        \
    } while (0)

static __m128 __lccc_simd128_ps_fmadd132ps128(long imm, __m128 a, __m128 b, __m128 c) {
    (void)imm;
    FMA_STANDIN_PS128(132, a, b, c, fma_standin_r128);
    return fma_standin_r128;
}
static __m128 __lccc_simd128_ps_fmadd213ps128(long imm, __m128 a, __m128 b, __m128 c) {
    (void)imm;
    FMA_STANDIN_PS128(213, a, b, c, fma_standin_r128);
    return fma_standin_r128;
}
static __m128 __lccc_simd128_ps_fmadd231ps128(long imm, __m128 a, __m128 b, __m128 c) {
    (void)imm;
    FMA_STANDIN_PS128(231, a, b, c, fma_standin_r128);
    return fma_standin_r128;
}

#define FMA_STANDIN_PD128(digits, a, b, c, out)                                  \
    do {                                                                        \
        __m128d s2 = (c), s3 = (b), d = (a);                                     \
        if ((digits) == 132) {                                                   \
            out = d * s3 + s2;                                                   \
        } else if ((digits) == 213) {                                            \
            out = s2 * d + s3;                                                   \
        } else {                                                                 \
            out = s2 * s3 + d;                                                   \
        }                                                                        \
    } while (0)

static __m128d __lccc_simd128_ps_fmadd132pd128(long imm, __m128d a, __m128d b, __m128d c) {
    (void)imm;
    FMA_STANDIN_PD128(132, a, b, c, fma_standin_r128d);
    return fma_standin_r128d;
}
static __m128d __lccc_simd128_ps_fmadd213pd128(long imm, __m128d a, __m128d b, __m128d c) {
    (void)imm;
    FMA_STANDIN_PD128(213, a, b, c, fma_standin_r128d);
    return fma_standin_r128d;
}
static __m128d __lccc_simd128_ps_fmadd231pd128(long imm, __m128d a, __m128d b, __m128d c) {
    (void)imm;
    FMA_STANDIN_PD128(231, a, b, c, fma_standin_r128d);
    return fma_standin_r128d;
}

#define FMA_STANDIN_PS256(digits, a, b, c, out)                                  \
    do {                                                                        \
        __m256 s2 = (c), s3 = (b), d = (a);                                      \
        if ((digits) == 132) {                                                   \
            out = d * s3 + s2;                                                   \
        } else if ((digits) == 213) {                                            \
            out = s2 * d + s3;                                                   \
        } else {                                                                 \
            out = s2 * s3 + d;                                                   \
        }                                                                        \
    } while (0)

/* The 256-bit builtins return `void *` per the shipped header. */
static void *__lccc_simd256_ps_vfmadd132ps256(long imm, __m256 a, __m256 b, __m256 c) {
    (void)imm;
    FMA_STANDIN_PS256(132, a, b, c, fma_standin_r256);
    return &fma_standin_r256;
}
static void *__lccc_simd256_ps_vfmadd213ps256(long imm, __m256 a, __m256 b, __m256 c) {
    (void)imm;
    FMA_STANDIN_PS256(213, a, b, c, fma_standin_r256);
    return &fma_standin_r256;
}
static void *__lccc_simd256_ps_vfmadd231ps256(long imm, __m256 a, __m256 b, __m256 c) {
    (void)imm;
    FMA_STANDIN_PS256(231, a, b, c, fma_standin_r256);
    return &fma_standin_r256;
}

#define FMA_STANDIN_PD256(digits, a, b, c, out)                                  \
    do {                                                                        \
        __m256d s2 = (c), s3 = (b), d = (a);                                     \
        if ((digits) == 132) {                                                   \
            out = d * s3 + s2;                                                   \
        } else if ((digits) == 213) {                                            \
            out = s2 * d + s3;                                                   \
        } else {                                                                 \
            out = s2 * s3 + d;                                                   \
        }                                                                        \
    } while (0)

static void *__lccc_simd256_ps_vfmadd132pd256(long imm, __m256d a, __m256d b, __m256d c) {
    (void)imm;
    FMA_STANDIN_PD256(132, a, b, c, fma_standin_r256d);
    return &fma_standin_r256d;
}
static void *__lccc_simd256_ps_vfmadd213pd256(long imm, __m256d a, __m256d b, __m256d c) {
    (void)imm;
    FMA_STANDIN_PD256(213, a, b, c, fma_standin_r256d);
    return &fma_standin_r256d;
}
static void *__lccc_simd256_ps_vfmadd231pd256(long imm, __m256d a, __m256d b, __m256d c) {
    (void)imm;
    FMA_STANDIN_PD256(231, a, b, c, fma_standin_r256d);
    return &fma_standin_r256d;
}

#if defined(__AVX512F__)
/* 512-bit: the identical pre-fix assignment.  The store bug here was separate
 * in code (`evex_store_dest` hard-coded `%zmm0`) but identical in shape, so
 * the same emulation applies and the 213/231 rows must disagree the same way. */
#define FMA_STANDIN_PS512(digits, a, b, c, out)                                  \
    do {                                                                        \
        __m512 s2 = (c), s3 = (b), d = (a);                                      \
        if ((digits) == 132) {                                                   \
            out = d * s3 + s2;                                                   \
        } else if ((digits) == 213) {                                            \
            out = s2 * d + s3;                                                   \
        } else {                                                                 \
            out = s2 * s3 + d;                                                   \
        }                                                                        \
    } while (0)

static void *__lccc_simd512_ps_vfmadd132ps512(long imm, void *a, void *b, void *c) {
    (void)imm;
    FMA_STANDIN_PS512(132, *(__m512 *)a, *(__m512 *)b, *(__m512 *)c, fma_standin_r512);
    return &fma_standin_r512;
}
static void *__lccc_simd512_ps_vfmadd213ps512(long imm, void *a, void *b, void *c) {
    (void)imm;
    FMA_STANDIN_PS512(213, *(__m512 *)a, *(__m512 *)b, *(__m512 *)c, fma_standin_r512);
    return &fma_standin_r512;
}
static void *__lccc_simd512_ps_vfmadd231ps512(long imm, void *a, void *b, void *c) {
    (void)imm;
    FMA_STANDIN_PS512(231, *(__m512 *)a, *(__m512 *)b, *(__m512 *)c, fma_standin_r512);
    return &fma_standin_r512;
}

#define FMA_STANDIN_PD512(digits, a, b, c, out)                                  \
    do {                                                                        \
        __m512d s2 = (c), s3 = (b), d = (a);                                     \
        if ((digits) == 132) {                                                   \
            out = d * s3 + s2;                                                   \
        } else if ((digits) == 213) {                                            \
            out = s2 * d + s3;                                                   \
        } else {                                                                 \
            out = s2 * s3 + d;                                                   \
        }                                                                        \
    } while (0)

static void *__lccc_simd512_ps_vfmadd132pd512(long imm, void *a, void *b, void *c) {
    (void)imm;
    FMA_STANDIN_PD512(132, *(__m512d *)a, *(__m512d *)b, *(__m512d *)c, fma_standin_r512d);
    return &fma_standin_r512d;
}
static void *__lccc_simd512_ps_vfmadd213pd512(long imm, void *a, void *b, void *c) {
    (void)imm;
    FMA_STANDIN_PD512(213, *(__m512d *)a, *(__m512d *)b, *(__m512d *)c, fma_standin_r512d);
    return &fma_standin_r512d;
}
static void *__lccc_simd512_ps_vfmadd231pd512(long imm, void *a, void *b, void *c) {
    (void)imm;
    FMA_STANDIN_PD512(231, *(__m512d *)a, *(__m512d *)b, *(__m512d *)c, fma_standin_r512d);
    return &fma_standin_r512d;
}
#endif /* __AVX512F__ */

#endif /* FMA_PREFIX_STANDIN_H */
