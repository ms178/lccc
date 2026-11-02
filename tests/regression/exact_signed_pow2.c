/* Exact signed power-of-two division must not bias negative multiples.
 * -fwrapv supplies the intentional modular arithmetic contract. The oracle
 * divisor is volatile, so it cannot undergo constant division lowering. */
#include <stdint.h>
#include <stdio.h>

#define FUNCTIONS(T, S) \
__attribute__((noinline)) T adjacent_##S(T x) { return (x * (x + 1)) / 2; } \
__attribute__((noinline)) T odd_sub_##S(T x) { return (x * (x - 7)) / 2; } \
__attribute__((noinline)) T odd_xor_##S(T x) { return (x * (x ^ 5)) / 2; } \
__attribute__((noinline)) T complement_##S(T x) { return (x * ~x) / 2; } \
__attribute__((noinline)) T masked_##S(T x) { return (x & (T)-16) / 16; } \
__attribute__((noinline)) T exact_rem_##S(T x) { return (x * (x + 1)) % 2; } \
__attribute__((noinline)) T inexact_##S(T x) { return (x * (x + 2)) / 2; } \
__attribute__((noinline)) T insufficient_##S(T x) { return (x * (x + 1)) / 4; }
FUNCTIONS(int32_t, w32)
FUNCTIONS(int64_t, w64)

#define CHECK(T, S, X) do { \
    T x = (T)(X); \
    volatile T two = 2, four = 4, sixteen = 16; \
    T a = x * (x + 1); \
    if (adjacent_##S(x) != a / two || odd_sub_##S(x) != (x * (x - 7)) / two || \
        odd_xor_##S(x) != (x * (x ^ 5)) / two || complement_##S(x) != (x * ~x) / two || \
        masked_##S(x) != (x & (T)-16) / sixteen || exact_rem_##S(x) != a % two || \
        inexact_##S(x) != (x * (x + 2)) / two || insufficient_##S(x) != a / four) return 1; \
} while (0)

/* This load is intentionally unrelated to the result. The ordered reciprocal
 * prefix must NOT swallow four observable integer reads per vector chunk. */
volatile int exact_recip_probe;
__attribute__((noinline))
double extra_volatile_load(const double *v, int n) {
    double sum = 0.0;
    for (int j = 0; j < n; ++j) {
        (void)exact_recip_probe;
        sum += (1.0 / (double)(j * 7 + 1)) * v[j];
    }
    return sum;
}

int main(void) {
    uint64_t state = 0x123456789abcdef0ULL;
    for (unsigned i = 0; i < 20000; ++i) {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        CHECK(int32_t, w32, state);
        CHECK(int64_t, w64, state);
    }
    static const uint64_t edges[] = {0, 1, 2, 3, 0x7fffffffULL, 0x80000000ULL,
        0xffffffffULL, 0x7fffffffffffffffULL, 0x8000000000000000ULL, ~0ULL};
    for (unsigned i = 0; i < sizeof edges / sizeof edges[0]; ++i) {
        CHECK(int32_t, w32, edges[i]);
        CHECK(int64_t, w64, edges[i]);
    }
    const double v[] = {1, 2, 3, 4, 5};
    if (extra_volatile_load(v, 0) != 0.0 || extra_volatile_load(v, 5) <= 0.0) return 2;
    puts("exact signed pow2: 320160 comparisons OK");
    return 0;
}
