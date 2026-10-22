/* Rare and deliberately biased masked branches with call-spanning state.
 * Every recurrence is unsigned; no signed overflow or FP reassociation oracle.
 */
#include <stdint.h>
#include <stdio.h>

static volatile uint64_t cold[6] = {11, 37, 101, 509, 1009, 65537};

__attribute__((noinline)) static uint64_t sample(uint64_t x, unsigned mode) {
    if (mode == 1) return 0;       /* always the zero pattern */
    if (mode == 2) return UINT64_MAX; /* never the zero pattern */
    return (x * UINT64_C(0x9e3779b97f4a7c15)) ^ (x >> 17);
}

#define DEFINE_UPDATE(NAME, MASK, PATTERN, TEST) \
__attribute__((noinline)) static uint64_t NAME(unsigned n, uint64_t seed, unsigned mode) { \
    uint64_t k0 = cold[0], k1 = cold[1], k2 = cold[2]; \
    uint64_t k3 = cold[3], k4 = cold[4], k5 = cold[5]; \
    uint64_t a = seed, b = seed + 1, c = seed + 2, d = seed + 3; \
    for (unsigned i = 0; i < n; ++i) { \
        uint64_t h = sample((uint64_t)i + seed, mode); \
        a += h; b ^= a + (b << 3); c += b ^ (c >> 5); \
        d = ((d << 7) | (d >> 57)) ^ c; \
        if ((h & (MASK)) TEST (PATTERN)) { \
            a ^= k0; b += k1; c ^= k2; d += k3; a += k4; d ^= k5; \
        } \
    } \
    return a ^ b ^ c ^ d ^ k0 ^ k1 ^ k2 ^ k3 ^ k4 ^ k5; \
}

DEFINE_UPDATE(zero_pattern, UINT64_C(255), 0, ==)
DEFINE_UPDATE(nonzero_pattern, UINT64_C(255), 0, !=)
DEFINE_UPDATE(scattered_pattern, UINT64_C(0x0101010101010101), UINT64_C(0x0001000000010000), ==)
DEFINE_UPDATE(dense_condition, UINT64_C(1), 0, ==)

int main(void) {
    const unsigned trips[] = {0, 1, 2, 255, 256, 257, 4096};
    const uint64_t seeds[] = {0, 1, UINT64_MAX, UINT64_C(0x8000000000000000)};
    uint64_t digest = 0;
    for (unsigned mode = 0; mode < 3; ++mode)
        for (unsigned s = 0; s < sizeof(seeds) / sizeof(seeds[0]); ++s)
            for (unsigned t = 0; t < sizeof(trips) / sizeof(trips[0]); ++t) {
                uint64_t r = zero_pattern(trips[t], seeds[s], mode);
                r ^= nonzero_pattern(trips[t], seeds[s], mode);
                r += scattered_pattern(trips[t], seeds[s], mode);
                r ^= dense_condition(trips[t], seeds[s], mode);
                digest = digest * 33 + r;
            }
    printf("sparse-mask-updates %016llx\n", (unsigned long long)digest);
    return 0;
}
