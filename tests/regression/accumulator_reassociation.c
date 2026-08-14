/*
 * loop-carried accumulator reassociation (Adler-32 pattern).
 *
 * `sum1 += b[i]; sum2 += sum1;` (unsigned) is reassociated into the closed
 * form sum2' = sum2 + N*sum1 + Σ (N-i)*b[i]. The transform must be
 * bit-exact (unsigned wraparound) for every unroll factor and input —
 * including sum values near UINT_MAX so the wraparound is exercised.
 */
#include <stdio.h>
#include <limits.h>

static unsigned adler8(const unsigned char *b, unsigned len, unsigned s1, unsigned s2) {
    while (len >= 8) {
        len -= 8;
        s1 += b[0]; s2 += s1;
        s1 += b[1]; s2 += s1;
        s1 += b[2]; s2 += s1;
        s1 += b[3]; s2 += s1;
        s1 += b[4]; s2 += s1;
        s1 += b[5]; s2 += s1;
        s1 += b[6]; s2 += s1;
        s1 += b[7]; s2 += s1;
        b += 8;
    }
    return s1 ^ (s2 << 16);
}
static unsigned adler16(const unsigned char *b, unsigned len, unsigned s1, unsigned s2) {
    while (len >= 16) {
        len -= 16;
        int i;
        for (i = 0; i < 16; i++) { s1 += b[i]; s2 += s1; }
        b += 16;
    }
    return s1 ^ (s2 << 16);
}
static unsigned long long adler64_8(const unsigned char *b, unsigned len,
                                    unsigned long long s1, unsigned long long s2) {
    while (len >= 8) {
        len -= 8;
        s1 += b[0]; s2 += s1;
        s1 += b[1]; s2 += s1;
        s1 += b[2]; s2 += s1;
        s1 += b[3]; s2 += s1;
        s1 += b[4]; s2 += s1;
        s1 += b[5]; s2 += s1;
        s1 += b[6]; s2 += s1;
        s1 += b[7]; s2 += s1;
        b += 8;
    }
    return s1 ^ (s2 << 32);
}
/* signed accumulation must NOT be reassociated (overflow UB) — behavior check */
static int signed_acc(const int *b, int len) {
    int s = 0, i;
    for (i = 0; i < len; i++) s += b[i];
    return s;
}

static unsigned long long mix(unsigned long long h, unsigned long long v) {
    return (h ^ v) * 0x9e3779b97f4a7c15ULL + 0xdeadbeefULL;
}

int main(void) {
    unsigned long long h = 1469598103934665603ULL;
    unsigned char b[128];
    int i, it;
    for (i = 0; i < 128; i++) b[i] = (unsigned char)(i * 13 + 7);

    for (it = 0; it < 200; it++) {
        unsigned s1 = (unsigned)(it * 65521 + 1) & 0xffff;
        unsigned s2 = (unsigned)(it * 99991) & 0xffff;
        h = mix(h, adler8(b, 128, s1, s2));
        h = mix(h, adler16(b, 128, s1, s2));
        h = mix(h, (unsigned)adler64_8(b, 128, s1, s2));
        b[it & 127] ^= (unsigned char)(it * 3 + 1);
    }
    /* wraparound stress: start sums near UINT_MAX */
    h = mix(h, adler8(b, 64, 0xfffffff0u, 0xfffffff8u));
    h = mix(h, adler16(b, 64, 0xfffffff0u, 0xfffffff8u));
    h = mix(h, (unsigned)adler64_8(b, 64, 0xfffffffffffffff0ULL, 0xfffffffffffffff8ULL));

    int ib[16];
    for (i = 0; i < 16; i++) ib[i] = (i & 1) ? i : -i;
    h = mix(h, (unsigned)signed_acc(ib, 16));
    printf("%llu\n", (unsigned long long)h);
    return 0;
}
