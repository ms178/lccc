/* SPDX-License-Identifier: Zlib
 * Altered standalone extract from zlib-ng 2.3.3, crc32_braid_comb.c and
 * crc32_braid_comb_p.h, commit 12731092979c6d07f42da27da673a9f6c7b13586.
 * Copyright (C) 1995-2022 Mark Adler.
 * Selected by ms178/archpkgbuilds f705b69e16028cb0699fbb1b1e657ef09e6bca23,
 * packages/zlib-ng/PKGBUILD. Source hashes and exact boundaries are recorded
 * in ../WORKLOAD_PROVENANCE.md. License: third_party_licenses/Zlib.txt.
 *
 * The polynomial multiply/exponentiation bodies are unchanged. Standalone
 * types, a renamed noinline entry, runtime power-table initialization and the
 * deterministic harness replace zlib-ng's includes, table and public wrappers.
 * This measures CRC concatenation, not byte-stream CRC or full compression.
 */
#include <stdint.h>
#include <stdio.h>

#define POLY UINT32_C(0xedb88320)
#ifndef PASSES
#define PASSES 262144U
#endif
static uint32_t x2n_table[32];

/* Return a(x) * b(x) modulo the reflected CRC polynomial. a must be nonzero. */
static uint32_t multmodp(uint32_t a, uint32_t b) {
    uint32_t m, p;
    m = (uint32_t)1 << 31;
    p = 0;
    for (;;) {
        if (a & m) {
            p ^= b;
            if ((a & (m - 1)) == 0)
                break;
        }
        m >>= 1;
        b = b & 1 ? (b >> 1) ^ POLY : b >> 1;
    }
    return p;
}

/* Return x^(n * 2^k) modulo p(x); n is nonnegative, table is initialized. */
static uint32_t x2nmodp(int64_t n, unsigned k) {
    uint32_t p;
    p = (uint32_t)1 << 31;
    while (n) {
        if (n & 1)
            p = multmodp(x2n_table[k & 31], p);
        n >>= 1;
        k++;
    }
    return p;
}

__attribute__((noinline)) uint32_t zng_crc32_combine(uint32_t crc1, uint32_t crc2,
                                                    int64_t len2) {
    return multmodp(x2nmodp(len2, 3), crc1) ^ crc2;
}

/* Independent reference: a bitwise byte CRC, with no polynomial multiplier. */
static uint32_t byte_crc(const unsigned char *data, unsigned n) {
    uint32_t crc = UINT32_MAX;
    for (unsigned i = 0; i < n; ++i) {
        crc ^= data[i];
        for (unsigned bit = 0; bit < 8; ++bit)
            crc = (crc >> 1) ^ ((0U - (crc & 1U)) & POLY);
    }
    return ~crc;
}

/* A separate 32x32 GF(2) linear-map reference covers lengths too large to
 * materialize. Start with the one-zero-byte transition, then square maps.
 * It does not use multmodp, x2nmodp or their precomputed powers. */
static uint32_t apply_map(const uint32_t map[32], uint32_t value) {
    uint32_t result = 0;
    for (unsigned bit = 0; bit < 32; ++bit)
        if ((value >> bit) & 1U)
            result ^= map[bit];
    return result;
}
static uint32_t reference_combine(uint32_t a, uint32_t b, uint64_t length) {
    uint32_t map[32], squared[32];
    for (unsigned bit = 0; bit < 32; ++bit) {
        uint32_t value = UINT32_C(1) << bit;
        for (unsigned step = 0; step < 8; ++step)
            value = (value >> 1) ^ ((0U - (value & 1U)) & POLY);
        map[bit] = value;
    }
    while (length) {
        if (length & 1U)
            a = apply_map(map, a);
        length >>= 1;
        if (!length)
            break;
        for (unsigned bit = 0; bit < 32; ++bit)
            squared[bit] = apply_map(map, map[bit]);
        for (unsigned bit = 0; bit < 32; ++bit)
            map[bit] = squared[bit];
    }
    return a ^ b;
}

int main(void) {
    unsigned char data[256];
    uint32_t checks[64];
    int64_t lengths[64];
    uint32_t state = UINT32_C(0x178);
    x2n_table[0] = UINT32_C(1) << 30;
    for (unsigned i = 1; i < 32; ++i)
        x2n_table[i] = multmodp(x2n_table[i - 1], x2n_table[i - 1]);
    for (unsigned i = 0; i < sizeof(data); ++i) {
        state = state * UINT32_C(1664525) + UINT32_C(1013904223);
        data[i] = (unsigned char)(state >> 24);
    }
    if (byte_crc((const unsigned char *)"123456789", 9) != UINT32_C(0xcbf43926))
        return 1;
    uint32_t whole = byte_crc(data, sizeof(data));
    for (unsigned split = 0; split <= sizeof(data); ++split) {
        uint32_t left = byte_crc(data, split);
        uint32_t right = byte_crc(data + split, sizeof(data) - split);
        if (zng_crc32_combine(left, right, sizeof(data) - split) != whole)
            return 2;
    }
    const int64_t edges[] = {
        0, 1, 2, 7, 255, 256, 65521, INT64_C(2147483647),
        INT64_C(4294967297), INT64_MAX
    };
    for (unsigned i = 0; i < sizeof(edges) / sizeof(edges[0]); ++i) {
        for (unsigned bit = 0; bit < 32; ++bit) {
            uint32_t a = UINT32_C(1) << bit;
            uint32_t b = whole ^ a;
            if (zng_crc32_combine(a, b, edges[i]) != reference_combine(a, b, edges[i]))
                return 3;
        }
    }
    for (unsigned i = 0; i < 64; ++i) {
        state = state * UINT32_C(1664525) + UINT32_C(1013904223);
        checks[i] = byte_crc(data, i * 4U);
        lengths[i] = state & UINT32_C(0xfffff);
    }
    uint32_t sum = whole;
    for (unsigned i = 0; i < PASSES; ++i) {
        unsigned k = (sum ^ i) & 63U;
        sum = zng_crc32_combine(sum, checks[k], lengths[k]);
    }
    printf("zlib_ng_crc32_combine: %08x\n", (unsigned)sum);
    return 0;
}
