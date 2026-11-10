/* Differential correctness for the lane-mask select (`VecBlendv*`).
 *
 * The defect (engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md, SA-10):
 * the SSE2-baseline lowering of `_mm_blendv_ps`/`_mm_blendv_pd` used the
 * bitwise `andps/andnps/orps` triple, which is a PER-BIT select, while the
 * Intel contract -- and the AVX `vblendvps`/`vblendvpd` one-instruction form
 * the same IR op takes on an AVX2 target -- is a PER-LANE select keyed on
 * each lane's SIGN BIT.  The two agree only for masks whose lanes are
 * all-ones or all-zeros, i.e. compare-produced masks.
 *
 * Every kernel below feeds a mask that is NOT compare-produced, in one of
 * four classes, and prints a checksum over the result:
 *
 *   sign_only  every lane's sign bit set, every other bit clear
 *              (0x80000000 / 0x8000000000000000).  Every LANE selects
 *              `true`, but the per-bit reading yields `false`'s low 31 bits
 *              ORed with `true`'s high bit -- so this row discriminates too,
 *              which a naive "sign-only masks are safe" reading would miss.
 *   low_bit    only bit 0 set in every lane -- per-BIT selects `true`,
 *              per-LANE must select `false`.  THE discriminating row.
 *   alt        alternating nibble patterns, mixed sign per lane
 *   mixed      guaranteed one lane sign-set and one sign-clear, so the row
 *              can never be vacuously equal to an all-selects row
 *
 * The invariance the gate asserts is the CONTRACT, not a hardcoded number:
 * `ps128_low_bit` must print the SAME checksum at -msse4.1 and at -mavx, and
 * that checksum must equal GCC's and Clang's.  A compiler with the defect
 * prints a different value for `low_bit` (and `alt`/`mixed`) on the non-AVX
 * target than on the AVX one, and disagrees with both references.
 *
 * Controls, so a green run cannot be explained by "the test does nothing":
 *
 *   ps128_cmpmask   a compare-produced mask.  Both select readings agree on
 *                   all-ones/all-zeros lanes, so this row must be correct both
 *                   before and after the fix on every target.  If it changes,
 *                   the fix is wrong.
 *   epi8_perbit     `_mm_blendv_epi8`, whose documented contract IS a per-BIT
 *                   select (include/smmintrin.h:52-66) and which
 *                   `pand/pandn/por` implements exactly.  This row must be
 *                   identical on every target and must NOT change when the
 *                   FP rows are fixed: canonicalizing it too would be a NEW
 *                   bug, and this row turns red if anyone does.
 *
 * Build:  <cc> -O2 [-msse4.1] [-mavx] this.c -o /tmp/bv
 * Run:    /tmp/bv
 *
 * No UB: no signed overflow (all arithmetic is on the bit pattern through
 * `uint32_t`/`uint64_t`), every load is fully initialised, no out-of-bounds
 * indexing, no strict-aliasing violation (`mix_bytes` goes through
 * `unsigned char`, which may alias anything), every vector value crosses a
 * type boundary through `memcpy` (never a pointer cast), and the PRNG is
 * xorshift64* so every compiler runs the identical sequence.  Warning-free
 * under `-Wall -Wextra -Werror`, including at -msse4.1 where the 256-bit
 * helpers are compiled out entirely.
 */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

static uint64_t mix_bytes(uint64_t h, const void *p, size_t n) {
    const unsigned char *b = (const unsigned char *)p;
    for (size_t i = 0; i < n; i++) {
        h ^= (uint64_t)b[i];
        h *= 1099511628211ULL;
    }
    return h;
}

/* `memcpy` is the only sanctioned way to move a vector value between types
 * here: it is exact, it is free, and it cannot be a strict-aliasing
 * violation.  One overload per width. */
static uint64_t mix_m128(uint64_t h, __m128 v) { return mix_bytes(h, &v, sizeof v); }
static uint64_t mix_m128i(uint64_t h, __m128i v) { return mix_bytes(h, &v, sizeof v); }

/* The mask classes, as raw bit patterns.  `cls` 3 ("mixed") pins lane 0's
 * sign bit clear and lane 1's set (and so on, alternating), which makes the
 * row structurally non-vacuous: a sign-bit select MUST pick lane 0 from
 * `false` and lane 1 from `true`. */
static __m128 mask_ps(int cls) {
    uint32_t w[4];
    for (int i = 0; i < 4; i++) {
        switch (cls) {
        case 0: w[i] = 0x80000000u; break;                            /* sign_only */
        case 1: w[i] = 0x00000001u; break;                            /* low_bit   */
        case 2:                                                       /* alt       */
            w[i] = (i & 1) ? 0xAAAA5555u : 0x5555AAAAu;
            break;
        default:                                                      /* mixed     */
            /* Deliberately the INVERSE sign pattern of `alt`, so the four
             * classes have four DISTINCT sign vectors:
             * [1,1,1,1] [0,0,0,0] [0,1,0,1] [1,0,1,0].  Two classes that
             * happened to select the same lanes would be one redundant test
             * wearing two names. */
            w[i] = (i & 1) ? 0x7FFFFFFFu : 0x80000000u;
            break;
        }
    }
    __m128 r;
    memcpy(&r, w, sizeof w);
    return r;
}

static __m128d mask_pd(int cls) {
    uint64_t w[2];
    for (int i = 0; i < 2; i++) {
        switch (cls) {
        case 0: w[i] = 0x8000000000000000ULL; break;
        case 1: w[i] = 0x0000000000000001ULL; break;
        case 2:
            w[i] = (i & 1) ? 0xAAAAAAAA55555555ULL : 0x55555555AAAAAAAAULL;
            break;
        default:
            /* qword sign bit is 63: alternate the TOP bit (not the low one,
             * which a qword lane must ignore) and use the inverse phase of
             * `alt`, so the two rows are genuinely different tests. */
            w[i] = (i & 1) ? 0x7FFFFFFFFFFFFFFFULL : 0x8000000000000000ULL;
            break;
        }
    }
    __m128d r;
    memcpy(&r, w, sizeof w);
    return r;
}

/* Fixed, fully-initialised payloads -- never the PRNG, so the mask class is
 * the only thing that varies between a row's four runs. */
static __m128 payload_ps_false(void) {
    const uint32_t w[4] = {0x11111111u, 0x22222222u, 0x33333333u, 0x44444444u};
    __m128 r;
    memcpy(&r, w, sizeof w);
    return r;
}

static __m128 payload_ps_true(void) {
    const uint32_t w[4] = {0xF0F0F0F0u, 0xF1F1F1F1u, 0xF2F2F2F2u, 0xF3F3F3F3u};
    __m128 r;
    memcpy(&r, w, sizeof w);
    return r;
}

static __m128d payload_pd_false(void) {
    const uint64_t w[2] = {0x0123456789ABCDEFULL, 0xFEDCBA9876543210ULL};
    __m128d r;
    memcpy(&r, w, sizeof w);
    return r;
}

static __m128d payload_pd_true(void) {
    const uint64_t w[2] = {0x0F0F0F0F0F0F0F0FULL, 0x00FF00FF00FF00FFULL};
    __m128d r;
    memcpy(&r, w, sizeof w);
    return r;
}

/* 1-4. The 128-bit FP dword blend, one row per mask class.  `noinline` keeps
 * each blend a real call to the intrinsic rather than something a
 * const-folder can see through. */
__attribute__((noinline)) static __m128 blend_ps_class(int cls) {
    __m128 a = payload_ps_false();
    __m128 b = payload_ps_true();
    return _mm_blendv_ps(a, b, mask_ps(cls));
}

/* 5-8. The 128-bit FP qword blend: a qword's sign bit is 63, so this is the
 * row that distinguishes a correct two-step broadcast from a dword-only one
 * (which would read bit 31 of the HIGH dword and so get the right answer
 * only by accident on some masks). */
__attribute__((noinline)) static __m128d blend_pd_class(int cls) {
    __m128d a = payload_pd_false();
    __m128d b = payload_pd_true();
    return _mm_blendv_pd(a, b, mask_pd(cls));
}

#if defined(__AVX__)
static uint64_t mix_m256(uint64_t h, __m256 v) { return mix_bytes(h, &v, sizeof v); }

static __m256 mask_ps256(int cls) {
    uint32_t w[8];
    for (int i = 0; i < 8; i++) {
        switch (cls) {
        case 0: w[i] = 0x80000000u; break;
        case 1: w[i] = 0x00000001u; break;
        case 2: w[i] = (i & 1) ? 0xAAAA5555u : 0x5555AAAAu; break;
        default: w[i] = (i & 1) ? 0x7FFFFFFFu : 0x80000000u; break;
        }
    }
    __m256 r;
    memcpy(&r, w, sizeof w);
    return r;
}

static __m256d mask_pd256(int cls) {
    uint64_t w[4];
    for (int i = 0; i < 4; i++) {
        switch (cls) {
        case 0: w[i] = 0x8000000000000000ULL; break;
        case 1: w[i] = 0x0000000000000001ULL; break;
        case 2: w[i] = (i & 1) ? 0xAAAAAAAA55555555ULL : 0x55555555AAAAAAAAULL; break;
        default: w[i] = (i & 1) ? 0x7FFFFFFFFFFFFFFFULL : 0x8000000000000000ULL; break;
        }
    }
    __m256d r;
    memcpy(&r, w, sizeof w);
    return r;
}

static __m256 payload_ps256_false(void) {
    uint32_t w[8];
    for (int i = 0; i < 8; i++) w[i] = 0x11111111u * (uint32_t)((i & 3) + 1);
    __m256 r;
    memcpy(&r, w, sizeof w);
    return r;
}

static __m256 payload_ps256_true(void) {
    uint32_t w[8];
    for (int i = 0; i < 8; i++) w[i] = 0xF0F0F0F0u ^ (uint32_t)((i & 3) * 0x01010101u);
    __m256 r;
    memcpy(&r, w, sizeof w);
    return r;
}

/* 9-12 / 13-16. The same two families at 256 bits: a different lowering with
 * the same contract, included so a fix that only touched the 128-bit path is
 * caught immediately. */
__attribute__((noinline)) static __m256 blend_ps256_class(int cls) {
    __m256 a = payload_ps256_false();
    __m256 b = payload_ps256_true();
    return _mm256_blendv_ps(a, b, mask_ps256(cls));
}

__attribute__((noinline)) static __m256d blend_pd256_class(int cls) {
    const uint64_t fa[4] = {0x0123456789ABCDEFULL, 0xFEDCBA9876543210ULL,
                            0x1111222233334444ULL, 0x5555666677778888ULL};
    const uint64_t tb[4] = {0x0F0F0F0F0F0F0F0FULL, 0x00FF00FF00FF00FFULL,
                            0x3333333333333333ULL, 0xCCCCCCCCCCCCCCCCULL};
    __m256d a, b;
    memcpy(&a, fa, sizeof fa);
    memcpy(&b, tb, sizeof tb);
    return _mm256_blendv_pd(a, b, mask_pd256(cls));
}
#endif /* __AVX__ */

/* 17. A compare-produced mask: the CONTROL.  Correct before and after the
 * fix on every target, because both select readings agree on an
 * all-ones/all-zeros mask.  If this row ever changes, the fix is wrong. */
__attribute__((noinline)) static __m128 blend_ps_compare_mask(void) {
    __m128 a = payload_ps_false();
    __m128 b = payload_ps_true();
    /* lanes 1 and 3 all-ones, lanes 0 and 2 all-zeros */
    __m128 m = _mm_castsi128_ps(
        _mm_cmpgt_epi32(_mm_setr_epi32(1, 0, 1, 1), _mm_setr_epi32(0, 2, 0, 0)));
    return _mm_blendv_ps(a, b, m);
}

/* 18. `_mm_blendv_epi8`: the explicit NON-regression control.  Its documented
 * contract is a per-BIT select (include/smmintrin.h:52-66), which
 * `pand/pandn/por` implements exactly, so this row must be identical on every
 * target and must NOT change when the FP rows are fixed. */
__attribute__((noinline)) static __m128i blend_epi8_perbit(void) {
    __m128i a = _mm_setr_epi8(0x01, 0x23, 0x45, 0x67, (char)0x89, (char)0xAB,
                              (char)0xCD, (char)0xEF, 0x11, 0x22, 0x33, 0x44,
                              0x55, 0x66, 0x77, 0x78);
    __m128i b = _mm_setr_epi8((char)0xFE, (char)0xDC, (char)0xBA, (char)0x98,
                              0x76, 0x54, 0x32, 0x10, (char)0xEE, (char)0xDD,
                              (char)0xCC, (char)0xBB, (char)0xAA, 0x19, 0x18,
                              0x17);
    /* 0x0F nibble mask: selects `b` in the low nibble of every byte and `a` in
     * the high nibble.  A per-BYTE-sign reading would select all of `a`
     * (not one payload bit 7 is set); a per-BIT reading produces the
     * interleave.  This is the row that proves the integer path is a
     * genuinely different contract. */
    __m128i m = _mm_set1_epi8(0x0F);
    return _mm_blendv_epi8(a, b, m);
}

int main(void) {
    uint64_t h = 1469598103934665603ULL;
    static const char *cls_name[4] = {"sign_only", "low_bit", "alt", "mixed"};

    for (int c = 0; c < 4; c++) {
        printf("ps128_%-9s %016llx\n", cls_name[c],
               (unsigned long long)mix_m128(h, blend_ps_class(c)));
    }
    for (int c = 0; c < 4; c++) {
        printf("pd128_%-9s %016llx\n", cls_name[c],
               (unsigned long long)mix_m128(h,
                   _mm_castpd_ps(blend_pd_class(c))));
    }
#if defined(__AVX__)
    for (int c = 0; c < 4; c++) {
        printf("ps256_%-9s %016llx\n", cls_name[c],
               (unsigned long long)mix_m256(h, blend_ps256_class(c)));
    }
    for (int c = 0; c < 4; c++) {
        printf("pd256_%-9s %016llx\n", cls_name[c],
               (unsigned long long)mix_m256(h,
                   _mm256_castpd_ps(blend_pd256_class(c))));
    }
#endif
    printf("ps128_%-9s %016llx\n", "cmpmask",
           (unsigned long long)mix_m128(h, blend_ps_compare_mask()));
    printf("epi8_%-10s %016llx\n", "perbit",
           (unsigned long long)mix_m128i(h, blend_epi8_perbit()));
    return 0;
}
