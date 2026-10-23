/*
 * A negative dynamic signed bound is a zero-trip loop, not a large unsigned
 * vector trip count. The matmul FMA transform scales grouped bounds into byte
 * offsets; unsigned division or width overflow can turn a negative value into
 * a positive packed-loop limit and cause out-of-bounds memory accesses.
 *
 * The runtime cases straddle the 32-bit and 64-bit wrap boundaries for the
 * grouped byte limit, as well as ordinary signed extremes and positive tails.
 * The companion check script compiles I32/I64 bounds through both the AVX2
 * byte-offset transform and the forced two-wide transform, and asserts that
 * each binary still contains packed arithmetic so a scalar fallback cannot
 * make the negative cases pass vacuously.
 */
#include <limits.h>
#include <stdio.h>

#define ELEMENTS 64

#ifdef TEST_I64_BOUND
typedef long long bound_t;
static const bound_t test_bounds[] = {
    LLONG_MIN,
    LLONG_MIN + 1,
    -2305843009213693953LL,
    -2305843009213693952LL,
    -2305843009213693951LL,
    -1152921504606846978LL,
    -1152921504606846977LL,
    -1152921504606846976LL,
    -1152921504606846975LL,
    -17, -16, -1, 0, 1, 4, 15, 16, 17, 63, 64,
};
#else
typedef int bound_t;
static const bound_t test_bounds[] = {
    INT_MIN,
    INT_MIN + 1,
    -536870913, -536870912, -536870911,
    -268435458, -268435457, -268435456, -268435455,
    -17, -16, -1, 0, 1, 4, 15, 16, 17, 63, 64,
};
#endif

static double factor;
static double source[ELEMENTS];
static double dest[ELEMENTS];

__attribute__((noinline)) static void matmul_vector_loop(bound_t n)
{
    for (bound_t i = 0; i < n; ++i)
        dest[i] += factor * source[i];
}

static void reset(void)
{
    factor = 2.0;
    for (int i = 0; i < ELEMENTS; ++i) {
        source[i] = (double)(3 * i + 1);
        dest[i] = (double)(100 - i);
    }
}

int main(void)
{
    for (size_t case_idx = 0;
         case_idx < sizeof(test_bounds) / sizeof(test_bounds[0]);
         ++case_idx) {
        bound_t n = test_bounds[case_idx];
        reset();
        matmul_vector_loop(n);

        for (int i = 0; i < ELEMENTS; ++i) {
            double want = (double)(100 - i);
            if (n > (bound_t)i)
                want += 2.0 * (double)(3 * i + 1);
            if (dest[i] != want) {
                printf("FAIL n=%lld i=%d got=%.17g want=%.17g\n",
                       (long long)n, i, dest[i], want);
                return 1;
            }
        }
    }
    puts("OK");
    return 0;
}
