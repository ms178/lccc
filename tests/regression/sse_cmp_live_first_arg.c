/* Regression: a packed compare, shuffle or round whose FIRST operand is still
 * needed after the op.
 *
 * The compare, shuffle and round emitters used %xmm0 as their working register.
 * When the first operand was an argument in %xmm0, the destructive op
 * overwrote it, and a later use of that operand read the op's result. For
 * example, _mm_move_sd(a, _mm_cmpneq_pd(a, b)) returned the compare's high
 * lane instead of a's. The emitters now compute in %xmm2.
 *
 * Each function below keeps its first argument live past the op. The output is
 * printed as raw bits and must match GCC 16.2 at -O1 and -O2. */
#include <immintrin.h>
#include <stdio.h>

__attribute__((noinline)) __m128d cmp_then_move(__m128d a, __m128d b)
{
    return _mm_move_sd(a, _mm_cmpneq_pd(a, b));
}

__attribute__((noinline)) __m128d cmp_then_move_named(__m128d a, __m128d b)
{
    __m128d c = _mm_cmpneq_pd(a, b);
    return _mm_move_sd(a, c);
}

__attribute__((noinline)) __m128d cmp_lt_then_or(__m128d a, __m128d b)
{
    __m128d c = _mm_cmp_pd(a, b, _CMP_LT_OQ);
    return _mm_or_pd(c, a);
}

__attribute__((noinline)) __m128d shuffle_then_add(__m128d a, __m128d b)
{
    __m128d s = _mm_shuffle_pd(a, b, 1);
    return _mm_add_pd(s, a);
}

__attribute__((noinline)) __m128 cmp_ps_then_move(__m128 a, __m128 b)
{
    __m128 c = _mm_cmp_ps(a, b, _CMP_GE_OQ);
    return _mm_move_ss(a, c);
}

__attribute__((noinline)) __m128d scalar_compare(__m128d a, __m128d b)
{
    return _mm_cmp_sd(a, b, _CMP_NLT_UQ);
}

static void dump_pd(const char *tag, __m128d v)
{
    unsigned long long r[2];
    _mm_storeu_si128((__m128i *)r, _mm_castpd_si128(v));
    printf("%s %016llx %016llx\n", tag, r[0], r[1]);
}

static void dump_ps(const char *tag, __m128 v)
{
    unsigned int r[4];
    _mm_storeu_si128((__m128i *)r, _mm_castps_si128(v));
    printf("%s %08x %08x %08x %08x\n", tag, r[0], r[1], r[2], r[3]);
}

int main(void)
{
    __m128d a = _mm_setr_pd(1.0, 40.0);
    __m128d b = _mm_setr_pd(2.0, 2.0);
    __m128d c = _mm_setr_pd(2.0, 7.5);
    __m128 pa = _mm_setr_ps(1.0f, 40.0f, 3.0f, 4.0f);
    __m128 pb = _mm_setr_ps(2.0f, 2.0f, 5.0f, 0.5f);

    dump_pd("cmp_then_move", cmp_then_move(a, b));
    dump_pd("cmp_then_move_named", cmp_then_move_named(a, b));
    dump_pd("cmp_lt_then_or", cmp_lt_then_or(a, b));
    dump_pd("cmp_lt_then_or_c", cmp_lt_then_or(c, b));
    dump_pd("shuffle_then_add", shuffle_then_add(a, b));
    dump_ps("cmp_ps_then_move", cmp_ps_then_move(pa, pb));
    dump_pd("scalar_compare", scalar_compare(a, c));
    return 0;
}
