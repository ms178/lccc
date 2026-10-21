/* Call-spanning hot recurrences competing with cold, non-rematerializable
 * values. Rare-path reads keep those values from pure-invariant demotion.
 * Both the result and cold values must survive register-home eviction. */
#include <stdint.h>

static volatile uint64_t salt = UINT64_C(0x9e3779b97f4a7c15);
static volatile uint64_t cold[6] = { 11, 23, 47, 97, 193, 389 };

__attribute__((noinline)) static uint64_t helper(uint64_t x) {
    return (x ^ salt) * UINT64_C(0xd6e8feb86659fd93);
}

__attribute__((noinline)) static uint64_t exercise(unsigned n, uint64_t seed) {
    uint64_t p = cold[0], q = cold[1], r = cold[2];
    uint64_t s = cold[3], t = cold[4], u = cold[5];
    uint64_t a = seed, b = seed + 1, c = seed + 2, d = seed + 3;
    for (unsigned i = 0; i < n; ++i) {
        uint64_t h = helper(i);
        a += h;
        b ^= a + (b << 3);
        c += b ^ (c >> 5);
        d = (d << 7) | (d >> 57);
        d ^= c;
        if ((h & 255) == 0) {
            a ^= p; b += q; c ^= r; d += s;
            a += t; d ^= u;
        }
    }
    return a ^ b ^ c ^ d ^ p ^ q ^ r ^ s ^ t ^ u;
}

/* Memory-carried reference with the same unsigned operations. */
__attribute__((noinline)) static uint64_t reference(unsigned n, uint64_t seed) {
    volatile uint64_t v[4] = { seed, seed + 1, seed + 2, seed + 3 };
    uint64_t p[6];
    for (unsigned j = 0; j < 6; ++j) p[j] = cold[j];
    for (unsigned i = 0; i < n; ++i) {
        uint64_t h = helper(i);
        v[0] += h;
        v[1] ^= v[0] + (v[1] << 3);
        v[2] += v[1] ^ (v[2] >> 5);
        v[3] = (v[3] << 7) | (v[3] >> 57);
        v[3] ^= v[2];
        if ((h & 255) == 0) {
            v[0] ^= p[0]; v[1] += p[1]; v[2] ^= p[2];
            v[3] += p[3]; v[0] += p[4]; v[3] ^= p[5];
        }
    }
    return v[0] ^ v[1] ^ v[2] ^ v[3] ^ p[0] ^ p[1] ^ p[2] ^ p[3] ^ p[4] ^ p[5];
}

int main(void) {
    const unsigned counts[] = { 0, 1, 2, 3, 17, 255, 256, 257, 1025 };
    const uint64_t seeds[] = { 0, 1, UINT64_MAX, UINT64_C(0x8000000000000000) };
    for (unsigned k = 0; k < 4; ++k) {
        salt ^= UINT64_C(0x1020304050607081);
        for (unsigned j = 0; j < 6; ++j) cold[j] = (cold[j] << 1) ^ salt;
        for (unsigned i = 0; i < sizeof(counts) / sizeof(counts[0]); ++i)
            for (unsigned j = 0; j < sizeof(seeds) / sizeof(seeds[0]); ++j)
                if (exercise(counts[i], seeds[j]) != reference(counts[i], seeds[j]))
                    return 1;
    }
    return 0;
}
