/* A deferred vector result may remain in %xmm0 until the next intrinsic.
 * More live scalar FP values than allocatable XMM homes force later scalar
 * operations through the %xmm0 scratch path.  Carrying the deferred value
 * across those operations used the last scalar result as the vector input. */
#include <immintrin.h>
#include <stdint.h>

static volatile double sink;

__attribute__((noinline))
static int check(int seed, double a) {
    __m128i mask = _mm_set1_epi32(5);
    __m128i value = _mm_set1_epi32(seed);
    double y0 = a + 1.25;
    double y1 = a + 2.25;
    double y2 = a + 3.25;
    double y3 = a + 4.25;
    double y4 = a + 5.25;
    double y5 = a + 6.25;
    double y6 = a + 7.25;
    double y7 = a + 8.25;
    double y8 = a + 9.25;
    double y9 = a + 10.25;
    double y10 = a + 11.25;
    double y11 = a + 12.25;
    double y12 = a + 13.25;
    double y13 = a + 14.25;
    double y14 = a + 15.25;
    double y15 = a + 16.25;
    double y16 = a + 17.25;
    double y17 = a + 18.25;
    double y18 = a + 19.25;
    double y19 = a + 20.25;
    __m128i result = _mm_xor_si128(value, mask);
    int lane0 = _mm_cvtsi128_si32(result);
    sink = y0 + y1 + y2 + y3 + y4 + y5 + y6 + y7 + y8 + y9 + y10 + y11 + y12 + y13 + y14 + y15 + y16 + y17 + y18 + y19;
    return lane0 == (seed ^ 5);
}

int main(void) {
    for (int i = 1; i < 1000; ++i) {
        if (!check(i, (double)i * 1.25)) return 1;
    }
    return 0;
}
