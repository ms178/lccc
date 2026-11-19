/* SSE4.1 / SSE3 / SSE2 intrinsics that were missing from lccc's headers
 * (GCC 16.2 accepts them under -mavx):
 *
 *   SSE4.1 round   _mm_round_ss/sd, _mm_ceil/floor_{ps,pd,ss,sd}
 *   SSE4.1 test    _mm_testc_si128, _mm_testnzc_si128, _mm_test_all_ones,
 *                  _mm_test_all_zeros, _mm_test_mix_ones_zeros
 *   SSE4.2         _mm_cmpgt_epi64
 *   AVX scalar cmp _mm_cmp_ss, _mm_cmp_sd
 *   SSE3           _mm_loaddup_pd, _mm_lddqu_si128
 *   SSE/SSE2       _mm_loadl_pi, _mm_loadh_pi, _mm_storel_pi, _mm_storeh_pi
 *                  (__m64_u operands), _mm_loadu/storeu_si{16,32,64},
 *                  _mm_maskmoveu_si128, _mm_move_epi16, _mm_move_epi32,
 *                  _mm_undefined_si128
 *
 * Every result is printed as raw bits, so a wrong sign of zero, a wrong NaN
 * payload, a lane taken from the wrong operand, or a byte written outside a
 * mask changes the output. The binary must print the same bytes as GCC 16.2.
 * _mm_monitor and _mm_mwait are privileged, so they are only compiled, never
 * called. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

static void dump_ps(const char *tag, __m128 v)
{
    unsigned int r[4];
    _mm_storeu_si128((__m128i *)r, _mm_castps_si128(v));
    printf("%s %08x %08x %08x %08x\n", tag, r[0], r[1], r[2], r[3]);
}

static void dump_pd(const char *tag, __m128d v)
{
    unsigned long long r[2];
    _mm_storeu_si128((__m128i *)r, _mm_castpd_si128(v));
    printf("%s %016llx %016llx\n", tag, r[0], r[1]);
}

static void dump_si(const char *tag, __m128i v)
{
    unsigned long long r[2];
    _mm_storeu_si128((__m128i *)r, v);
    printf("%s %016llx %016llx\n", tag, r[0], r[1]);
}

static float f32_from_bits(unsigned int b)
{
    float f;
    memcpy(&f, &b, 4);
    return f;
}

static double f64_from_bits(unsigned long long b)
{
    double d;
    memcpy(&d, &b, 8);
    return d;
}

static void round_cases(void)
{
    /* -0.0, +0.0, halves, a NaN, +inf, -inf, large magnitude, negatives. */
    __m128 pv = _mm_setr_ps(-2.5f, 3.75f, f32_from_bits(0x80000000u), f32_from_bits(0x7fc00001u));
    __m128 pw = _mm_setr_ps(f32_from_bits(0x7f800000u), f32_from_bits(0xff800000u), -0.4f, 1.0e30f);
    __m128d dv = _mm_setr_pd(-2.5, f64_from_bits(0x7ff8000000000001ULL));
    __m128d dw = _mm_setr_pd(f64_from_bits(0xfff0000000000000ULL), -0.25);
    __m128 dst_ps = _mm_setr_ps(10.0f, 20.0f, 30.0f, 40.0f);
    __m128d dst_pd = _mm_setr_pd(7.0, 8.0);

    dump_ps("round_ps_nearest", _mm_round_ps(pv, _MM_FROUND_TO_NEAREST_INT));
    dump_ps("round_ps_floor", _mm_round_ps(pw, _MM_FROUND_TO_NEG_INF));
    dump_ps("ceil_ps", _mm_ceil_ps(pv));
    dump_ps("floor_ps", _mm_floor_ps(pw));
    dump_pd("round_pd_trunc", _mm_round_pd(dv, _MM_FROUND_TO_ZERO));
    dump_pd("ceil_pd", _mm_ceil_pd(dw));
    dump_pd("floor_pd", _mm_floor_pd(dv));

    dump_ps("round_ss_ceil", _mm_round_ss(dst_ps, pv, _MM_FROUND_CEIL));
    dump_ps("ceil_ss", _mm_ceil_ss(dst_ps, pw));
    dump_ps("floor_ss", _mm_floor_ss(dst_ps, pv));
    dump_pd("round_sd_floor", _mm_round_sd(dst_pd, dv, _MM_FROUND_FLOOR));
    dump_pd("ceil_sd", _mm_ceil_sd(dst_pd, dw));
    dump_pd("floor_sd", _mm_floor_sd(dst_pd, dw));
}

static void test_cases(void)
{
    __m128i zero = _mm_setzero_si128();
    __m128i ones = _mm_set1_epi32(-1);
    __m128i a = _mm_set_epi64x(0x00000000ffff0000LL, 0x0f0f0f0f0f0f0f0fLL);
    __m128i b = _mm_set_epi64x(0x0000ffff00000000LL, 0xf0f0f0f0f0f0f0f0LL);
    __m128i half = _mm_set_epi64x(0x0000000000000000LL, 0x00000000ffffffffLL);

    printf("testc_ones_ones %d\n", _mm_testc_si128(ones, ones));
    printf("testc_zero_ones %d\n", _mm_testc_si128(zero, ones));
    printf("testc_a_b %d\n", _mm_testc_si128(a, b));
    printf("testc_b_a %d\n", _mm_testc_si128(b, a));
    printf("testnzc_a_b %d\n", _mm_testnzc_si128(a, b));
    printf("testnzc_a_a %d\n", _mm_testnzc_si128(a, a));
    printf("testnzc_zero %d\n", _mm_testnzc_si128(zero, half));
    printf("testz_a_b %d\n", _mm_testz_si128(a, b));
    printf("all_ones_ones %d\n", _mm_test_all_ones(ones));
    printf("all_ones_half %d\n", _mm_test_all_ones(half));
    printf("all_zeros_a_b %d\n", _mm_test_all_zeros(a, b));
    printf("all_zeros_a_half %d\n", _mm_test_all_zeros(a, half));
    printf("mix_a_b %d\n", _mm_test_mix_ones_zeros(a, b));
    printf("mix_zero %d\n", _mm_test_mix_ones_zeros(zero, ones));

    /* Signed 64-bit compare: the sign bit of a lane decides the order. */
    __m128i p = _mm_set_epi64x(-1LL, 5LL);
    __m128i q = _mm_set_epi64x(-2LL, 5LL);
    dump_si("cmpgt_epi64", _mm_cmpgt_epi64(p, q));
    dump_si("cmpgt_epi64_rev", _mm_cmpgt_epi64(q, p));
    dump_si("cmpgt_epi64_min", _mm_cmpgt_epi64(_mm_set_epi64x(0LL, (long long)0x8000000000000000ULL), _mm_set_epi64x(0LL, 0LL)));
}

static void scalar_cmp_cases(void)
{
    __m128 sa = _mm_setr_ps(1.0f, -2.0f, 3.0f, -4.0f);
    __m128 sb = _mm_setr_ps(2.0f, -2.0f, 1.0f, 9.0f);
    __m128d da = _mm_setr_pd(1.5, -0.0);
    __m128d db = _mm_setr_pd(1.5, 0.0);

    /* predicate 1 = LT_OS, 4 = NEQ_UQ, 13 = GE_OS, 0 = EQ_OQ. The upper
     * lanes come from the first operand, whatever the predicate. */
    dump_ps("cmp_ss_lt", _mm_cmp_ss(sa, sb, 1));
    dump_ps("cmp_ss_neq", _mm_cmp_ss(sa, sb, 4));
    dump_ps("cmp_ss_ge", _mm_cmp_ss(sb, sa, 13));
    dump_ps("cmp_ss_eq", _mm_cmp_ss(sa, sa, 0));
    dump_pd("cmp_sd_eq", _mm_cmp_sd(da, db, 0));
    dump_pd("cmp_sd_lt", _mm_cmp_sd(da, db, 1));
    dump_pd("cmp_sd_neq", _mm_cmp_sd(db, da, 4));
}

static void load_store_cases(void)
{
    double dbl[4] = {1.25, -0.0, 3.5, 4.75};
    __m128d dup = _mm_loaddup_pd(&dbl[2]);
    dump_pd("loaddup_pd", dup);

    unsigned char src[32];
    for (int i = 0; i < 32; i++)
        src[i] = (unsigned char)(0xa0 + i);
    dump_si("lddqu_unaligned", _mm_lddqu_si128((const __m128i *)(src + 1)));

    /* Unaligned 8-byte MMX operands. */
    dump_ps("loadl_pi", _mm_loadl_pi(_mm_setr_ps(9.0f, 8.0f, 7.0f, 6.0f), (const __m64_u *)(src + 3)));
    dump_ps("loadh_pi", _mm_loadh_pi(_mm_setr_ps(9.0f, 8.0f, 7.0f, 6.0f), (const __m64_u *)(src + 5)));

    unsigned char out[24];
    memset(out, 0x55, sizeof out);
    _mm_storel_pi((__m64_u *)(out + 1), _mm_setr_ps(1.0f, 2.0f, 3.0f, 4.0f));
    _mm_storeh_pi((__m64_u *)(out + 9), _mm_setr_ps(1.0f, 2.0f, 3.0f, 4.0f));
    for (int i = 0; i < 24; i++)
        printf("%02x", out[i]);
    printf("\n");

    /* Partial loads zero the upper bytes; partial stores write only their own
     * width. */
    dump_si("loadu_si16", _mm_loadu_si16(src + 7));
    dump_si("loadu_si32", _mm_loadu_si32(src + 3));
    dump_si("loadu_si64", _mm_loadu_si64(src + 9));

    unsigned char st[24];
    memset(st, 0x77, sizeof st);
    _mm_storeu_si16(st + 1, _mm_set_epi64x(0x1122334455667788LL, 0x99aabbccddeeff01LL));
    _mm_storeu_si32(st + 6, _mm_set_epi64x(0x1122334455667788LL, 0x99aabbccddeeff01LL));
    _mm_storeu_si64(st + 12, _mm_set_epi64x(0x1122334455667788LL, 0x99aabbccddeeff01LL));
    for (int i = 0; i < 24; i++)
        printf("%02x", st[i]);
    printf("\n");

    /* MASKMOVDQU: only bytes whose mask sign bit is set are written. */
    unsigned char mem[16];
    memset(mem, 0x2e, sizeof mem);
    __m128i data = _mm_set_epi8(15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0);
    __m128i mask = _mm_set_epi8(-1, 0, 0x7f, -128, 0, 1, -2, 0, 0x80, 0x40, -1, 0, 0, 0, 0, -1);
    _mm_maskmoveu_si128(data, mask, (char *)mem);
    for (int i = 0; i < 16; i++)
        printf("%02x", mem[i]);
    printf("\n");

    dump_si("move_epi16", _mm_move_epi16(_mm_set_epi64x(0x1122334455667788LL, 0x99aabbccddeeff01LL)));
    dump_si("move_epi32", _mm_move_epi32(_mm_set_epi64x(0x1122334455667788LL, 0x99aabbccddeeff01LL)));
}

/* Compiled, never called: MONITOR and MWAIT are privileged. */
static void never_called_privileged(void *p)
{
    _mm_monitor(p, 0, 0);
    _mm_mwait(0, 0);
}

int main(void)
{
    round_cases();
    test_cases();
    scalar_cmp_cases();
    load_store_cases();
    if (0)
        never_called_privileged(NULL);
    printf("done\n");
    return 0;
}
