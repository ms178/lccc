/* Block placement is a frequency hint, never a uniform-input assumption.
   Exercise all-zero, never-zero, alternating and sparse-zero populations. */
#include <stdint.h>
#include <stdio.h>
__attribute__((noinline)) uint64_t diamonds(const uint64_t *p, int n, uint64_t seed) {
    for (int i = 0; i < n; ++i) {
        uint64_t x = p[i];
        seed = seed * 1664525u + 1013904223u;
        if ((x & 255) == 0) seed ^= x + (seed >> 7);
        if ((x & 15) != 0) seed += x ^ (seed << 3);
        if ((x & 65535) == 0) seed = (seed << 7) | (seed >> 57);
        if ((x & 7) == 0) seed ^= 0xdeadbeefU;
    }
    return seed;
}
int main(void) {
    uint64_t input[1024];
    for (int mode = 0; mode < 4; ++mode) {
        for (int i = 0; i < 1024; ++i) {
            uint64_t x = (uint64_t)(unsigned)i * UINT64_C(0xd6e8feb86659fd93);
            if (mode == 0) x &= UINT64_C(0xffffffffffff0000);
            if (mode == 1) x |= 1;
            if (mode == 2) x = (i & 1) ? x | 1 : x & UINT64_C(0xffffffffffff0000);
            input[i] = x;
        }
        for (int n = -1; n <= 1024; ++n)
            printf("%d %d %016llx\n", mode, n,
                (unsigned long long)diamonds(input, n, UINT64_C(0x123456789abcdef0)));
    }
    return 0;
}
