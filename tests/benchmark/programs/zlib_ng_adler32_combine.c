/* zlib-ng 2.3.3: adler32.c, adler32_combine_ (Zlib license).
 * Copyright (C) 1995-2011, 2016 Mark Adler.
 * Selected from ms178/archpkgbuilds d9953b4f185fe6b33506af4b5730039af7849174,
 * packages/zlib-ng/PKGBUILD (pkgver=2.3.3).
 * https://github.com/zlib-ng/zlib-ng/blob/2.3.3/adler32.c
 * Source SHA-256: bc5308ff3ea584bf13b626e39ad261abbaf7f87e6eb92a9195c8cef70bc0bf9d
 * Adaptation: standalone stdint types, renamed noinline entry, deterministic
 * harness. Arithmetic body unchanged. License: third_party_licenses/Zlib.txt.
 * Measures checksum composition/modular arithmetic, NOT full compression.
 */
#include <stdint.h>
#include <stdio.h>
#define BASE 65521U
#ifndef PASSES
#define PASSES 50000000U
#endif

__attribute__((noinline)) uint32_t zng_combine(uint32_t adler1, uint32_t adler2, int64_t len2) {
    uint32_t sum1;
    uint32_t sum2;
    unsigned rem;

    /* for negative len, return invalid adler32 as a clue for debugging */
    if (len2 < 0)
        return 0xffffffff;

    /* the derivation of this formula is left as an exercise for the reader */
    len2 %= BASE;                 /* assumes len2 >= 0 */
    rem = (unsigned)len2;
    sum1 = adler1 & 0xffff;
    sum2 = rem * sum1;
    sum2 %= BASE;
    sum1 += (adler2 & 0xffff) + BASE - 1;
    sum2 += ((adler1 >> 16) & 0xffff) + ((adler2 >> 16) & 0xffff) + BASE - rem;
    if (sum1 >= BASE) sum1 -= BASE;
    if (sum1 >= BASE) sum1 -= BASE;
    if (sum2 >= ((unsigned long)BASE << 1)) sum2 -= ((unsigned long)BASE << 1);
    if (sum2 >= BASE) sum2 -= BASE;
    return sum1 | (sum2 << 16);
}


static unsigned char data[1024];
static uint32_t checks[64];
static uint32_t reference(const unsigned char *p, unsigned int n) {
    uint32_t a = 1, b = 0;
    for (unsigned int i = 0; i < n; ++i) { a = (a + p[i]) % BASE; b = (b + a) % BASE; }
    return (b << 16) | a;
}
int main(void) {
    for (unsigned int i = 0; i < sizeof(data); ++i) data[i] = (unsigned char)(i*37U + 11U);
    uint32_t whole = reference(data, sizeof(data));
    for (unsigned int split = 0; split <= sizeof(data); ++split) {
        uint32_t left = reference(data, split);
        uint32_t right = reference(data + split, sizeof(data) - split);
        if (zng_combine(left, right, sizeof(data) - split) != whole) return 1;
    }
    if (zng_combine(1, 1, -1) != UINT32_MAX) return 2;
    /* Arbitrarily long runs of zero bytes: a=1, b=length mod BASE. */
    const int64_t lengths[] = {0, 1, 65520, 65521, 65522, 2147483648LL, 9223372036854775807LL};
    for (unsigned int i = 0; i < sizeof(lengths)/sizeof(lengths[0]); ++i) {
        uint32_t zero = ((uint32_t)(lengths[i] % BASE) << 16) | 1;
        if (zng_combine(1, zero, lengths[i]) != zero) return 3;
    }
    for (unsigned int i = 0; i < 64; ++i) checks[i] = reference(data, i*16U);
    uint32_t sum = 1;
    for (unsigned int i = 0; i < PASSES; ++i) {
        unsigned int k = (sum ^ i) & 63U;
        sum = zng_combine(sum, checks[k], k*16U);
    }
    printf("zlib_ng_adler32_combine: %08x\n", (unsigned int)sum);
    return 0;
}
