/* Shifted runtime thresholds are bounded, but their input may wrap.
 * All memory accesses are masked independently of the optimization's proof.
 * Breaks bound execution even for deliberately wrapping do-while seeds.
 */
#include <stdint.h>
#include <limits.h>
#include <stdio.h>

static volatile int bounds[] = {INT_MIN, INT_MIN + 1, -100, -2, -1, 0, 1, 2, 17, INT_MAX - 1, INT_MAX};
static volatile int seeds[] = {0, 1, 7, -1, INT_MAX};
static uint32_t data[16];

__attribute__((noinline)) static uint32_t guarded(int n) {
    uint32_t sum = 0;
    int bound = (n + 1) >> 1;
    for (int i = 0; i < bound; ++i) {
        uint32_t j = (uint32_t)i & 15u;
        data[j] ^= (uint32_t)(i + 100);
        sum += data[j];
        if (i == 127) break;
    }
    return sum;
}
__attribute__((noinline)) static uint32_t exposed(int n, int seed) {
    uint32_t sum = 0;
    unsigned count = 0;
    int i = seed;
    do {
        uint32_t j = (uint32_t)i & 15u;
        data[j] ^= (uint32_t)(i + 100);
        sum += data[j];
        if (++count == 128) break;
        ++i;
    } while (i < ((n + 1) >> 1));
    return sum;
}
int main(void) {
    uint32_t digest = 0;
    for (unsigned n = 0; n < sizeof(bounds)/sizeof(bounds[0]); ++n) {
        digest = (digest << 5) | (digest >> 27);
        digest ^= guarded(bounds[n]);
        for (unsigned s = 0; s < sizeof(seeds)/sizeof(seeds[0]); ++s)
            digest = digest * 33u + exposed(bounds[n], seeds[s]);
    }
    printf("shift-bound %08x\n", (unsigned)digest);
    return 0;
}
