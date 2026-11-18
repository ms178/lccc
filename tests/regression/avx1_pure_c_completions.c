/* AVX1 pure-C header completions under -mavx: set/setr (ps, pd, epi16,
 * epi64x, m128 forms), zext, set_m128*, loadu2/storeu2, movedup/movehdup/
 * moveldup, _mm_permute_ps/_mm_permute_pd, permute2f128_si256, cvt scalars,
 * and undefined/zeroupper.
 *
 * Every expected value is a constant written from the Intel definition (lane
 * order: set_* lists the highest lane first, setr_* the lowest first, and
 * set_m128*(hi, lo) puts lo in the low half). Results are compared bit for bit
 * with memcmp, so a lane swap, a wrong zero half, or a wrong sign fails. The
 * test is self-contained and needs no reference compiler at run time.
 * Compiled with -O2 -mavx; exit status is nonzero on any mismatch.
 */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static int failures;

#define CHECK_BITS(name, got, exp)                                      \
    do {                                                                \
        if (memcmp(&(got), (exp), sizeof(got)) != 0) {                  \
            printf("FAIL %s\n", name);                                  \
            failures++;                                                 \
        } else {                                                        \
            printf("ok   %s\n", name);                                  \
        }                                                               \
    } while (0)

int main(void)
{
    /* setr_ps: lowest lane first. */
    const float f_seq[8] = {0, 1, 2, 3, 4, 5, 6, 7};
    __m256 f = _mm256_setr_ps(0, 1, 2, 3, 4, 5, 6, 7);
    CHECK_BITS("setr_ps", f, f_seq);

    /* set_ps and set_m128 with differing halves. */
    const float g_exp[8] = {1, 1, 1, 1, 9, 9, 9, 9};
    __m256 g = _mm256_set_m128(_mm_set1_ps(9.0f), _mm_set1_ps(1.0f));
    CHECK_BITS("set_m128", g, g_exp);

    /* zext: upper four lanes must be zero, not undefined. */
    const float z_exp[8] = {2, 2, 2, 2, 0, 0, 0, 0};
    __m256 z = _mm256_zextps128_ps256(_mm_set1_ps(2.0f));
    CHECK_BITS("zextps128_ps256", z, z_exp);

    /* setr_pd and set_m128d(hi, lo). */
    const double d_exp[4] = {1, 2, 3, 4};
    __m256d d = _mm256_setr_pd(1, 2, 3, 4);
    CHECK_BITS("setr_pd", d, d_exp);
    const double dd_exp[4] = {8, 8, 7, 7};
    __m256d dd = _mm256_set_m128d(_mm_set1_pd(7.0), _mm_set1_pd(8.0));
    CHECK_BITS("set_m128d", dd, dd_exp);

    /* zextpd128: upper two lanes must be zero. */
    const double zd_exp[4] = {5, 5, 0, 0};
    __m256d zd = _mm256_zextpd128_pd256(_mm_set1_pd(5.0));
    CHECK_BITS("zextpd128_pd256", zd, zd_exp);

    /* setr_epi16 and set_epi16 (reversed order). */
    const short i16_exp[16] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15};
    __m256i i16 = _mm256_setr_epi16(0, 1, 2, 3, 4, 5, 6, 7,
                                    8, 9, 10, 11, 12, 13, 14, 15);
    CHECK_BITS("setr_epi16", i16, i16_exp);
    __m256i s16 = _mm256_set_epi16(15, 14, 13, 12, 11, 10, 9, 8,
                                   7, 6, 5, 4, 3, 2, 1, 0);
    CHECK_BITS("set_epi16", s16, i16_exp);

    /* setr_epi64x. */
    const long long q_exp[4] = {10, 11, 12, 13};
    __m256i q = _mm256_setr_epi64x(10, 11, 12, 13);
    CHECK_BITS("setr_epi64x", q, q_exp);

    /* setr_m128i(lo, hi). */
    const int m_exp[8] = {3, 3, 3, 3, 4, 4, 4, 4};
    __m256i m = _mm256_setr_m128i(_mm_set1_epi32(3), _mm_set1_epi32(4));
    CHECK_BITS("setr_m128i", m, m_exp);

    /* loadu2: first pointer is the high half, second the low half. */
    float fa[8] = {0, 1, 2, 3, 4, 5, 6, 7};
    const float lu_exp[8] = {0, 1, 2, 3, 4, 5, 6, 7};
    __m256 lu = _mm256_loadu2_m128(fa + 4, fa);
    CHECK_BITS("loadu2_m128", lu, lu_exp);

    /* storeu2: high half to the first pointer, low half to the second. */
    float ho[4] = {0}, lo[4] = {0};
    const float ho_exp[4] = {14, 15, 16, 17};
    const float lo_exp[4] = {10, 11, 12, 13};
    __m256 st = _mm256_setr_ps(10, 11, 12, 13, 14, 15, 16, 17);
    _mm256_storeu2_m128(ho, lo, st);
    CHECK_BITS("storeu2_m128 hi", ho, ho_exp);
    CHECK_BITS("storeu2_m128 lo", lo, lo_exp);

    /* movehdup / moveldup / movedup. */
    const float mh_exp[8] = {1, 1, 3, 3, 5, 5, 7, 7};
    const float ml_exp[8] = {0, 0, 2, 2, 4, 4, 6, 6};
    const double md_exp[4] = {1, 1, 3, 3};
    __m256 mh = _mm256_movehdup_ps(f);
    __m256 ml = _mm256_moveldup_ps(f);
    __m256d md = _mm256_movedup_pd(d);
    CHECK_BITS("movehdup_ps", mh, mh_exp);
    CHECK_BITS("moveldup_ps", ml, ml_exp);
    CHECK_BITS("movedup_pd", md, md_exp);

    /* In-lane permutes with immediates. imm 0x1b reverses four floats; for
     * pd, imm 0x1 selects lane 1 into lane 0 and lane 0 into lane 1. */
    const float pp_exp[4] = {3, 2, 1, 0};
    __m128 pp = _mm_permute_ps(_mm_setr_ps(0, 1, 2, 3), 0x1b);
    CHECK_BITS("permute_ps", pp, pp_exp);
    const double pd_exp[2] = {1, 0};
    __m128d pd = _mm_permute_pd(_mm_setr_pd(0, 1), 0x1);
    CHECK_BITS("permute_pd", pd, pd_exp);

    /* permute2f128_si256: imm 0x21 selects a.hi into the low half and b.lo
     * into the high half, so the result is {a[2], a[3], b[0], b[1]} in
     * 64-bit lanes. */
    __m256i q_hi = _mm256_setr_epi64x(10, 11, 12, 13);
    __m256i p2 = _mm256_permute2f128_si256(q_hi, m, 0x21);
    const long long p2_q[4] = {12, 13, 3 | ((long long)3 << 32), 3 | ((long long)3 << 32)};
    CHECK_BITS("permute2f128_si256", p2, p2_q);

    /* scalar conversions. */
    if (_mm256_cvtss_f32(f) != 0.0f) { printf("FAIL cvtss_f32\n"); failures++; }
    else printf("ok   cvtss_f32\n");
    if (_mm256_cvtsd_f64(d) != 1.0) { printf("FAIL cvtsd_f64\n"); failures++; }
    else printf("ok   cvtsd_f64\n");
    if (_mm256_cvtsi256_si32(m) != 3) { printf("FAIL cvtsi256_si32\n"); failures++; }
    else printf("ok   cvtsi256_si32\n");

    /* undefined_* must produce a value without side effects; zeroupper must
     * be callable. */
    __m256 ud = _mm256_undefined_ps();
    (void)ud;
    _mm256_zeroupper();
    printf("ok   undefined_zeroupper\n");

    if (failures) {
        printf("%d failure(s)\n", failures);
        return 1;
    }
    printf("all AVX1 pure-C completion checks passed\n");
    return 0;
}
