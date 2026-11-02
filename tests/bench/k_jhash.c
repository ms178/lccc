/* Linux kernel jhash (Jenkins hash, include/linux/jhash.h algorithm shape):
 * the __jhash_mix triple of subtract/xor/shift rounds over 12-byte blocks.
 * Pure dependent ALU chains with no memory traffic in the hot loop after the
 * loads: the purest possible latency-chain / dual-issue probe, and a hash
 * that really runs in networking and dcache paths.  Modelled on the public
 * Jenkins lookup3 mixing function; finalisation truncated to the mix so the
 * timed region is the loop, not the tail switch. */
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "bench.h"

#define N 4096
static unsigned char buf[N];

void bench_setup(void) {
    uint32_t s = 0xDEADBEEFu;
    for (int i = 0; i < N; i++) { s = s * 1103515245u + 12345u; buf[i] = (unsigned char)(s >> 16); }
}

#define MIX(a, b, c) do { \
    a -= b; a -= c; a ^= (c >> 13); \
    b -= c; b -= a; b ^= (a << 8);  \
    c -= a; c -= b; c ^= (b >> 13); \
    a -= b; a -= c; a ^= (c >> 12); \
    b -= c; b -= a; b ^= (a << 16); \
    c -= a; c -= b; c ^= (b >> 5);  \
    a -= b; a -= c; a ^= (c >> 3);  \
    b -= c; b -= a; b ^= (a << 10); \
    c -= a; c -= b; c ^= (b >> 15); \
} while (0)

static uint32_t jhash_blocks(const unsigned char *b, size_t len, uint32_t seed) {
    uint32_t a = 0x9E3779B9u, bb = 0x9E3779B9u, c = seed;
    size_t i = 0;
    for (; i + 12 <= len; i += 12) {
        uint32_t x, y, z;
        memcpy(&x, b + i, 4); memcpy(&y, b + i + 4, 4); memcpy(&z, b + i + 8, 4);
        a += x; bb += y; c += z;
        MIX(a, bb, c);
    }
    c += (uint32_t)len;
    for (; i < len; i++) { a += b[i]; MIX(a, bb, c); }
    return c;
}

int bench_verify(void) {
    /* Expected values computed from an independent transliteration of the
     * MIX macro (Python model, distinct implementation): a swapped shift
     * count in either this macro or a future edit diverges immediately.
     * Note this kernel's per-byte tail handling is its own shape, not the
     * kernel's packed-tail jhash, so published lookup3 vectors do not
     * apply -- the dual implementation is the oracle. */
    static const unsigned char zeros[12] = {0};
    unsigned char range24[24];
    for (int i = 0; i < 24; i++) range24[i] = (unsigned char)i;
    if (jhash_blocks(zeros, 12, 0) != 0xBD49D119u) return 1;
    if (jhash_blocks(range24, 24, 0x1234u) != 0x07C98B7Fu) return 2;
    if (jhash_blocks(zeros, 0, 0x1234u) != 0x1234u) return 3;
    return 0;
}

unsigned long long bench_run(void) { return jhash_blocks(buf, N, 0x1234u); }
