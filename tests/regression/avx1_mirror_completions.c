/* AVX1 mirror completions under -mavx: _mm256_addsub_pd, _mm256_dp_ps,
 * _mm256_rcp_ps, _mm256_rsqrt_ps and the 128-bit _mm_permutevar_ps/_pd.
 *
 * Every expected value is derived from the Intel definition, not from another
 * compiler's output:
 *   addsub_pd: even lanes a-b, odd lanes a+b.
 *   dp_ps:     per 128-bit half, sum of products selected by imm[7:4], written
 *              to the lanes selected by imm[3:0]; other lanes are +0.0.
 *   permutevar_ps: lane i = v[idx_i & 3].
 *   permutevar_pd: lane i = v[(idx_qword_i >> 1) & 1].
 *   rcp/rsqrt: documented relative error bound 1.5 * 2^-12.
 *
 * Each operation sits in its own noinline function so the check exercises the
 * instruction semantics alone. Exit status is nonzero on any mismatch.
 */
#include <immintrin.h>
#include <math.h>
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

#define CHECK_TRUE(name, cond)                                          \
    do {                                                                \
        if (!(cond)) {                                                  \
            printf("FAIL %s\n", name);                                  \
            failures++;                                                 \
        } else {                                                        \
            printf("ok   %s\n", name);                                  \
        }                                                               \
    } while (0)

__attribute__((noinline)) static __m256d addsub_pd(__m256d a, __m256d b)
{
    return _mm256_addsub_pd(a, b);
}

__attribute__((noinline)) static __m256 dp_ps(__m256 a, __m256 b)
{
    return _mm256_dp_ps(a, b, 0xF1);
}

__attribute__((noinline)) static __m256 dp_ps_mixed(__m256 a, __m256 b)
{
    /* imm 0x5A: high nibble 0x5 keeps products 0 and 2 of each half; low
     * nibble 0xA writes result lanes 1 and 3 of each half. Exercises a
     * non-trivial product mask and write mask on both halves. */
    return _mm256_dp_ps(a, b, 0x5A);
}

__attribute__((noinline)) static __m256 rcp_ps(__m256 a)
{
    return _mm256_rcp_ps(a);
}

__attribute__((noinline)) static __m256 rsqrt_ps(__m256 a)
{
    return _mm256_rsqrt_ps(a);
}

__attribute__((noinline)) static __m128 permutevar_ps(__m128 v, __m128i idx)
{
    return _mm_permutevar_ps(v, idx);
}

__attribute__((noinline)) static __m128d permutevar_pd(__m128d v, __m128i idx)
{
    return _mm_permutevar_pd(v, idx);
}

int main(void)
{
    /* addsub_pd: a = [5 6 7 8], b = [1 2 3 4] (lane 0 first). */
    {
        __m256d a = _mm256_set_pd(8.0, 7.0, 6.0, 5.0);
        __m256d b = _mm256_set_pd(4.0, 3.0, 2.0, 1.0);
        __m256d got = addsub_pd(a, b);
        double exp[4] = {4.0, 8.0, 4.0, 12.0};
        CHECK_BITS("addsub_pd", got, exp);
    }

    /* dp_ps, imm 0xF1: lane 0 and lane 4 receive the full 4-product sum 60;
     * all other lanes are +0.0. */
    {
        __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
        __m256 b = _mm256_set_ps(1, 2, 3, 4, 5, 6, 7, 8);
        __m256 got = dp_ps(a, b);
        float exp[8] = {60.0f, 0.0f, 0.0f, 0.0f, 60.0f, 0.0f, 0.0f, 0.0f};
        CHECK_BITS("dp_ps_f1", got, exp);
    }

    /* dp_ps, imm 0x5A: high nibble 0x5 = products 0 and 2 per half;
     * low nibble 0xA = write lanes 1 and 3 per half.
     * Low half: a = [1 2 3 4], b = [8 7 6 5] -> 1*8 + 3*6 = 26 (both
     * written lanes). High half: a = [5 6 7 8], b = [4 3 2 1] -> 5*4 + 7*2 = 34.
     * Result lanes: [0, 26, 0, 26, 0, 34, 0, 34]. */
    {
        __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
        __m256 b = _mm256_set_ps(1, 2, 3, 4, 5, 6, 7, 8);
        __m256 got = dp_ps_mixed(a, b);
        float exp[8] = {0.0f, 26.0f, 0.0f, 26.0f, 0.0f, 34.0f, 0.0f, 34.0f};
        CHECK_BITS("dp_ps_5a", got, exp);
    }

    /* rcp_ps and rsqrt_ps: documented relative error bound 1.5 * 2^-12. */
    {
        __m256 a = _mm256_set_ps(8, 7, 6, 5, 4, 3, 2, 1);
        float x[8], r[8], s[8];
        _mm256_storeu_ps(x, a);
        _mm256_storeu_ps(r, rcp_ps(a));
        _mm256_storeu_ps(s, rsqrt_ps(a));
        const double bound = 1.5 / 4096.0;
        int rcp_ok = 1;
        int rsqrt_ok = 1;
        for (int i = 0; i < 8; i++) {
            if (fabs((double)r[i] * (double)x[i] - 1.0) > bound) {
                rcp_ok = 0;
            }
            if (fabs((double)s[i] * sqrt((double)x[i]) - 1.0) > bound) {
                rsqrt_ok = 0;
            }
        }
        CHECK_TRUE("rcp_ps_bound", rcp_ok);
        CHECK_TRUE("rsqrt_ps_bound", rsqrt_ok);
    }

    /* permutevar_ps: v = [1 2 3 4], idx lanes = [3 2 1 0] -> [4 3 2 1]. */
    {
        __m128 v = _mm_set_ps(4.0f, 3.0f, 2.0f, 1.0f);
        __m128i idx = _mm_set_epi32(0, 1, 2, 3);
        __m128 got = permutevar_ps(v, idx);
        float exp[4] = {4.0f, 3.0f, 2.0f, 1.0f};
        CHECK_BITS("permutevar_ps", got, exp);
    }

    /* permutevar_pd: v = [1 2], idx qwords = [2 0]; bit 1 selects element.
     * Lane 0 takes v[1] = 2.0, lane 1 takes v[0] = 1.0. */
    {
        __m128d v = _mm_set_pd(2.0, 1.0);
        __m128i idx = _mm_set_epi64x(0, 2);
        __m128d got = permutevar_pd(v, idx);
        double exp[2] = {2.0, 1.0};
        CHECK_BITS("permutevar_pd", got, exp);
    }

    if (failures != 0) {
        printf("FAILURES: %d\n", failures);
        return 1;
    }
    printf("PASS avx1_mirror_completions\n");
    return 0;
}
