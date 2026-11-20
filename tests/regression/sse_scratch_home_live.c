/* Regression: a value that the destructive compare, shuffle or round stages
 * must survive the op.
 *
 * The packed compare, shuffle and round emitters write their result into
 * %xmm2 (the destructive scratch). Two cases must keep their operands:
 *   - a scalar double homed in %xmm2 and read again after the op (xmm2 is
 *     withheld from the F64 pool only when clobbers_xmm2 lists the op);
 *   - a vector parameter parked in %xmm2 by the ABI, read after a round.
 * A parameter parked in %xmm0 or %xmm1 and read after a compare is covered by
 * sse_cmp_live_first_arg.c.
 * Output must match GCC 16.2 at -O1 and -O2 (round needs -msse4.1).
 */
#include <immintrin.h>
#include <stdio.h>

/* Scalar doubles are homed in %xmm2..%xmm7 (the F64 pool). The packed compare
 * scratches %xmm2, so a double live across it must be withheld from %xmm2 or
 * the compare destroys it. Several doubles stay live across each compare. */
__attribute__((noinline)) double x2_cmp(double p, double q, double r, __m128d a, __m128d b)
{
    double s0 = p * q + 1.25;
    double s1 = q * r - 0.5;
    double s2 = p - r * 2.0;
    __m128d c = _mm_cmpnlt_pd(a, b);
    double m = _mm_cvtsd_f64(c);
    __m128d d = _mm_cmpneq_pd(b, a);
    double n = _mm_cvtsd_f64(_mm_shuffle_pd(d, c, 1));
    return s0 + s1 * s2 + p + q + r + m * 0.0 + n * 0.0;
}

__attribute__((noinline)) double x2_round(double p, double q, __m128d a)
{
    double s0 = p * q - 2.0;
    double s1 = p + q * 0.25;
    __m128d r = _mm_round_pd(a, 1);
    __m128d sh = _mm_shuffle_pd(a, a, 1);
    return s0 * s1 + p + q + _mm_cvtsd_f64(r) * 0.0 + _mm_cvtsd_f64(sh) * 0.0;
}

/* H1: the SECOND operand is a parameter parked in %xmm1 and used after the op.
 * The compare emitter loads its second operand into %xmm1; if the parameter's
 * home is %xmm1 and the write clobbers it, the later use reads the mask. */
__attribute__((noinline)) __m128d h1_cmp_second(__m128d a, __m128d b)
{
    __m128d c = _mm_cmpneq_pd(a, b);
    return _mm_add_pd(_mm_move_sd(c, b), b);
}

__attribute__((noinline)) __m128d h1_shuf_second(__m128d a, __m128d b)
{
    __m128d c = _mm_shuffle_pd(a, b, 2);
    return _mm_add_pd(c, b);
}

__attribute__((noinline)) __m128d h1_round_second(__m128d a, __m128d b)
{
    __m128d c = _mm_round_pd(b, 1);
    return _mm_add_pd(_mm_move_sd(a, c), b);
}

__attribute__((noinline)) __m128 h1_cmp_ps(__m128 a, __m128 b)
{
    __m128 c = _mm_cmpnlt_ps(a, b);
    return _mm_add_ps(c, b);
}


/* The count-in-XMM shifts stage their destructive result in %xmm2 too. A value
 * read after the shift must survive it: a double homed in %xmm2 and vector
 * operands that the shifts read. */
__attribute__((noinline)) double x2_shift(double p, double q, __m128i a, __m128i c)
{
    double s0 = p * q - 0.75;
    __m128i sh = _mm_sll_epi32(a, c);
    __m128i sr = _mm_sra_epi16(a, c);
    return s0 + p * 2.0 + q + (double)_mm_extract_epi32(sh, 1)
        + (double)_mm_extract_epi32(sr, 2) * 16.0 + (double)_mm_extract_epi32(sh, 3) * 0.0;
}

static void pd(const char *t, __m128d v)
{
    unsigned long long r[2];
    _mm_storeu_si128((__m128i *)r, _mm_castpd_si128(v));
    printf("%s %016llx %016llx\n", t, r[0], r[1]);
}

int main(void)
{
    double z[2] = {1.5, -2.5};
    __m128d a = _mm_loadu_pd(z), b = _mm_setr_pd(0.5, -8.0);
    printf("x2_cmp %.17g\n", x2_cmp(1.5, -2.25, 3.0, a, b));
    printf("x2_cmp2 %.17g\n", x2_cmp(-0.75, 4.0, 0.125, b, a));
    printf("x2_round %.17g\n", x2_round(2.0, 3.0, a));
    printf("x2_shift %.17g\n", x2_shift(1.5, -2.25, _mm_setr_epi32(1, -40, 3, 4), _mm_set_epi64x(0, 2)));
    pd("h1_cmp_second", h1_cmp_second(a, b));
    pd("h1_shuf_second", h1_shuf_second(a, b));
    pd("h1_round_second", h1_round_second(a, b));
    return 0;
}
