/* Multi-use SSE2/SSE4.1 intrinsic chain for RA-21 A/B screening.
 * The arithmetic is intentionally stable across iterations: this isolates
 * register-home vs stack-home code generation without data-dependent noise. */
#include <immintrin.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

__attribute__((noinline))
static uint64_t sat_kernel(const uint8_t *a, const uint8_t *b,
                           const uint8_t *c, uint8_t *out, unsigned iterations) {
    for (unsigned i = 0; i < iterations; ++i) {
        __m128i va = _mm_loadu_si128((const __m128i *)a);
        __m128i vb = _mm_loadu_si128((const __m128i *)b);
        __m128i vc = _mm_loadu_si128((const __m128i *)c);
        __m128i t = _mm_adds_epu8(va, vb);
        __m128i u = _mm_adds_epu8(t, vc);
        __m128i v = _mm_avg_epu8(t, u);
        __m128i w = _mm_sub_epi64(v, t);
        _mm_storeu_si128((__m128i *)out, _mm_xor_si128(w, u));
    }
    uint64_t lo, hi;
    memcpy(&lo, out, sizeof lo);
    memcpy(&hi, out + sizeof lo, sizeof hi);
    return lo ^ hi;
}

int main(int argc, char **argv) {
    unsigned iterations = 1000;
    if (argc > 1) {
        unsigned long parsed = strtoul(argv[1], NULL, 0);
        if (parsed == 0 || parsed > UINT32_MAX)
            return 2;
        iterations = (unsigned)parsed;
    }

    uint8_t a[16], b[16], c[16], out[16];
    for (int i = 0; i < 16; ++i) {
        a[i] = (uint8_t)(i * 17 + 3);
        b[i] = (uint8_t)(i * 11 + 5);
        c[i] = (uint8_t)(i * 7 + 9);
    }
    printf("%llu\n", (unsigned long long)sat_kernel(a, b, c, out, iterations));
    return 0;
}
