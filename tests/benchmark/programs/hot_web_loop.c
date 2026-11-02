/* Call-spanning hot recurrences competing with cold, non-rematerializable
 * values. Rare-path reads keep those values from pure-invariant demotion.
 * Both the result and cold values must survive register-home eviction. */
#include <stdint.h>
#include <stdio.h>

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

#ifndef HOT_WEB_TRIPS
#define HOT_WEB_TRIPS 40000000u
#endif

int main(void) {
    printf("%016llx\n", (unsigned long long)exercise(HOT_WEB_TRIPS, UINT64_C(0xffff000000000001)));
    return 0;
}
