/* SSE2 / MMX intrinsics that were missing from lccc's headers (GCC 16.2
 * accepts them under -mavx):
 *
 *   SSE2   _mm_cmpneq_sd, _mm_cmpnlt_sd, _mm_cmpnle_sd, _mm_cmpngt_sd,
 *          _mm_cmpnge_sd, _mm_cmpord_sd, _mm_cmpunord_sd
 *   SSE2   _mm_set_epi64, _mm_set1_epi64, _mm_setr_epi64,
 *          _mm_movepi64_pi64, _mm_movpi64_epi64
 *   MMX    _mm_cvtsi64x_si64, _mm_cvtsi64_si64x, _mm_set_pi64x,
 *          _mm_add_si64, _mm_sub_si64, _mm_mul_su32, _mm_sll_si64,
 *          _mm_srl_si64, _mm_sll_pi16, _mm_srl_pi16, _mm_sra_pi16,
 *          _mm_sll_pi32, _mm_srl_pi32, _mm_sra_pi32
 *
 * Results print as raw bits. Scalar compares are tested with NaN, signed
 * zeros and ordered values, so each predicate's true and false outcomes both
 * show, and the high lane shows whether it came from the first operand. Counts
 * are tested at 0, the element width, one above it, and all-ones. The binary
 * must print the same bytes as GCC 16.2. */
#include <immintrin.h>
#include <stdio.h>
#include <string.h>

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

static void dump_m64(const char *tag, __m64 v)
{
    printf("%s %016llx\n", tag, (unsigned long long)_mm_cvtsi64_si64x(v));
}

static double f64_from_bits(unsigned long long b)
{
    double d;
    memcpy(&d, &b, 8);
    return d;
}

static void sd_compare_cases(void)
{
    const double qnan = f64_from_bits(0x7ff8000000000001ULL);
    const double negz = f64_from_bits(0x8000000000000000ULL);
    __m128d lo_lt = _mm_setr_pd(1.0, 40.0);   /* low lane: 1.0 < 2.0 */
    __m128d lo_gt = _mm_setr_pd(3.0, 50.0);   /* low lane: 3.0 > 2.0 */
    __m128d lo_eq = _mm_setr_pd(2.0, 60.0);   /* low lane: equal */
    __m128d lo_nan = _mm_setr_pd(qnan, 70.0); /* low lane: unordered */
    __m128d lo_z = _mm_setr_pd(negz, 80.0);   /* low lane: -0.0 == +0.0 */
    __m128d two = _mm_setr_pd(2.0, 90.0);
    __m128d zero = _mm_setr_pd(0.0, 100.0);

    __m128d xs[5] = { lo_lt, lo_gt, lo_eq, lo_nan, lo_z };
    const char *names[5] = { "lt", "gt", "eq", "nan", "negz" };
    for (int i = 0; i < 5; i++) {
        char tag[64];
        snprintf(tag, sizeof tag, "neq_%s", names[i]);
        dump_pd(tag, _mm_cmpneq_sd(xs[i], two));
        snprintf(tag, sizeof tag, "nlt_%s", names[i]);
        dump_pd(tag, _mm_cmpnlt_sd(xs[i], two));
        snprintf(tag, sizeof tag, "nle_%s", names[i]);
        dump_pd(tag, _mm_cmpnle_sd(xs[i], two));
        snprintf(tag, sizeof tag, "ngt_%s", names[i]);
        dump_pd(tag, _mm_cmpngt_sd(xs[i], two));
        snprintf(tag, sizeof tag, "nge_%s", names[i]);
        dump_pd(tag, _mm_cmpnge_sd(xs[i], two));
        snprintf(tag, sizeof tag, "ord_%s", names[i]);
        dump_pd(tag, _mm_cmpord_sd(xs[i], two));
        snprintf(tag, sizeof tag, "unord_%s", names[i]);
        dump_pd(tag, _mm_cmpunord_sd(xs[i], two));
    }
    /* Operands swapped: the upper lane must still come from the first. */
    dump_pd("ngt_swapped", _mm_cmpngt_sd(two, lo_gt));
    dump_pd("nge_swapped", _mm_cmpnge_sd(zero, lo_z));
}

static void set_and_move_cases(void)
{
    __m64 hi = _mm_set_pi64x(0x1122334455667788LL);
    __m64 lo = _mm_cvtsi64x_si64(0x0fedcba987654321LL);
    dump_si("set_epi64", _mm_set_epi64(hi, lo));
    dump_si("setr_epi64", _mm_setr_epi64(hi, lo));
    dump_si("set1_epi64", _mm_set1_epi64(lo));
    dump_m64("movepi64_pi64", _mm_movepi64_pi64(_mm_set_epi64x(0x5a5a5a5a5a5a5a5aLL, 0x0102030405060708LL)));
    dump_si("movpi64_epi64", _mm_movpi64_epi64(lo));
}

static void scalar_mmx_cases(void)
{
    __m64 maxv = _mm_cvtsi64x_si64(0x7fffffffffffffffLL);
    __m64 minus1 = _mm_cvtsi64x_si64(-1LL);
    __m64 one = _mm_cvtsi64x_si64(1LL);
    /* Wrap-around: signed overflow must not occur or trap. */
    dump_m64("add_wrap", _mm_add_si64(maxv, one));
    dump_m64("sub_wrap", _mm_sub_si64(_mm_cvtsi64x_si64(0LL), one));
    dump_m64("add_minus1", _mm_add_si64(minus1, one));
    /* PMULUDQ: only the low dwords participate. */
    __m64 a = _mm_cvtsi64x_si64((long long)0xffffffff00000003ULL);
    __m64 b = _mm_cvtsi64x_si64((long long)0x00000000fffffff1ULL);
    dump_m64("mul_su32", _mm_mul_su32(a, b));
    dump_m64("mul_su32_max", _mm_mul_su32(minus1, minus1));

    static const unsigned long long counts[] = { 0, 1, 31, 63, 64, 0xffffffffffffffffULL };
    __m64 v = _mm_cvtsi64x_si64((long long)0x8001cafe12345678ULL);
    for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {
        char tag[64];
        __m64 c = _mm_cvtsi64x_si64((long long)counts[i]);
        snprintf(tag, sizeof tag, "sll_si64_%llx", counts[i]);
        dump_m64(tag, _mm_sll_si64(v, c));
        snprintf(tag, sizeof tag, "srl_si64_%llx", counts[i]);
        dump_m64(tag, _mm_srl_si64(v, c));
    }
}

static void packed_mmx_shift_cases(void)
{
    __m64 v16 = _mm_set_pi16(-32768, 32767, -3, 0x0102);
    __m64 v32 = _mm_set_pi32(0x80000001, -2);
    static const unsigned long long counts[] = { 0, 1, 15, 16, 31, 32, 0xffffffffffffffffULL };
    for (unsigned i = 0; i < sizeof counts / sizeof counts[0]; i++) {
        char tag[64];
        __m64 c = _mm_cvtsi64x_si64((long long)counts[i]);
        snprintf(tag, sizeof tag, "sll_pi16_%llx", counts[i]);
        dump_m64(tag, _mm_sll_pi16(v16, c));
        snprintf(tag, sizeof tag, "srl_pi16_%llx", counts[i]);
        dump_m64(tag, _mm_srl_pi16(v16, c));
        snprintf(tag, sizeof tag, "sra_pi16_%llx", counts[i]);
        dump_m64(tag, _mm_sra_pi16(v16, c));
        snprintf(tag, sizeof tag, "sll_pi32_%llx", counts[i]);
        dump_m64(tag, _mm_sll_pi32(v32, c));
        snprintf(tag, sizeof tag, "srl_pi32_%llx", counts[i]);
        dump_m64(tag, _mm_srl_pi32(v32, c));
        snprintf(tag, sizeof tag, "sra_pi32_%llx", counts[i]);
        dump_m64(tag, _mm_sra_pi32(v32, c));
    }
}

int main(void)
{
    sd_compare_cases();
    set_and_move_cases();
    scalar_mmx_cases();
    packed_mmx_shift_cases();
    _mm_empty();
    printf("done\n");
    return 0;
}
