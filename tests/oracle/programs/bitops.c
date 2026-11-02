/* Bit manipulation: rotates, the popcount idiom, byte swap, CRC, and the
 * masking that decides whether a value stays in a register or spills. */
#include <stdio.h>
#include <stdint.h>

/* Rotations are masked to 1..31 / 1..63.  `x << 32` and `x << 64` are
 * UNDEFINED even for unsigned types, so an unmasked rotate is not a
 * portability test, it is a coin toss: Clang 23 folds it one way, GCC and
 * ICX fold it another, and the program has no defined answer to compare
 * against.  Oracle disagreement on UB is not a compiler bug. */
static inline uint32_t rotl(uint32_t x, int r) { r &= 31; return r ? (x << r) | (x >> (32 - r)) : x; }
static inline uint64_t rotl64(uint64_t x, int r) { r &= 63; return r ? (x << r) | (x >> (64 - r)) : x; }
static int popcnt(uint64_t x) {
    int c = 0;
    while (x) { x &= x - 1; c++; }             /* the classic idiom */
    return c;
}
static uint32_t bswap32(uint32_t x) {
    x = ((x & 0xFFu) << 24) | ((x & 0xFF00u) << 8) |
        ((x >> 8) & 0xFF00u) | ((x >> 24) & 0xFFu);
    return x;
}
static uint32_t crc32ish(uint32_t *p, int n) {
    uint32_t c = 0xFFFFFFFFu;
    for (int i = 0; i < n; i++) {
        c ^= p[i];
        for (int k = 0; k < 32; k++)
            c = (c >> 1) ^ (0xEDB88320u & (uint32_t)(-(int32_t)(c & 1)));
    }
    return ~c;
}
static uint64_t bf(uint64_t x) {             /* bit-field style packing */
    return ((x & 1) << 63) | ((x & 3) << 61) | ((x & 7) << 58) |
           ((x & 0xF) << 54) | ((x & 0x1F) << 49);
}

int main(void) {
    uint32_t buf[64];
    uint64_t h = 0;
    for (int i = 0; i < 64; i++) buf[i] = (uint32_t)i * 2246822519u;
    for (int i = 0; i < 512; i++) {
        h ^= rotl((uint32_t)i, (i & 31) + 1);
        h = rotl64(h, (i & 63) + 1) ^ (uint64_t)popcnt(h);
        h += bswap32((uint32_t)h ^ (uint32_t)(h >> 32));
        h ^= bf((uint64_t)i * 0x9E3779B97F4A7C15ull);
    }
    printf("bitops %016llx\n", (unsigned long long)h);
    printf("bitops crc %08x\n", crc32ish(buf, 64));
    return 0;
}
