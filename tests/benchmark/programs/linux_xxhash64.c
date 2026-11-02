/* SPDX-License-Identifier: BSD-2-Clause OR GPL-2.0-only */
/*
 * Workload-derived kernel: Linux kernel 6.18.58 (linux-cachymod-6.18),
 * lib/xxhash.c, xxh64() (BSD-2-Clause OR GPL-2.0-only).
 *
 * Keeps the 4-lane 64-bit stripe loop (`xxh64_round`), `xxh64_merge_round`,
 * unaligned little-endian tail finalization, and `xxh64_avalanche`. Verifies
 * standard XXH64 test vectors (empty, 1-byte, 14-byte, 64-byte) before running
 * the multi-length hash workload.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef BUF_SIZE
#define BUF_SIZE (1U << 16)
#endif
#ifndef PASSES
#define PASSES 512U
#endif

#define XXH_PRIME64_1 0x9E3779B185EBCA87ULL
#define XXH_PRIME64_2 0xC2B2AE3D27D4EB4FULL
#define XXH_PRIME64_3 0x165667B19E3779F9ULL
#define XXH_PRIME64_4 0x85EBCA77C2B2AE63ULL
#define XXH_PRIME64_5 0x27D4EB2F165667C5ULL

static uint8_t data_buf[BUF_SIZE + 64U];

static inline uint64_t rol64(uint64_t x, unsigned int s)
{
    return (x << s) | (x >> (64U - s));
}

static inline uint32_t rol32(uint32_t x, unsigned int s)
{
    return (x << s) | (x >> (32U - s));
}

static inline uint32_t get_unaligned_le32(const uint8_t *p)
{
    uint32_t v;
    memcpy(&v, p, sizeof(v));
    return v;
}

static inline uint64_t get_unaligned_le64(const uint8_t *p)
{
    uint64_t v;
    memcpy(&v, p, sizeof(v));
    return v;
}

static inline uint64_t xxh64_round(uint64_t acc, uint64_t input)
{
    acc += input * XXH_PRIME64_2;
    acc = rol64(acc, 31);
    acc *= XXH_PRIME64_1;
    return acc;
}

static inline uint64_t xxh64_merge_round(uint64_t acc, uint64_t val)
{
    val = xxh64_round(0, val);
    acc ^= val;
    acc = acc * XXH_PRIME64_1 + XXH_PRIME64_4;
    return acc;
}

static inline uint64_t xxh64_avalanche(uint64_t h64)
{
    h64 ^= h64 >> 33;
    h64 *= XXH_PRIME64_2;
    h64 ^= h64 >> 29;
    h64 *= XXH_PRIME64_3;
    h64 ^= h64 >> 32;
    return h64;
}

__attribute__((noinline))
static uint64_t kernel_xxh64(const void *input, size_t len, uint64_t seed)
{
    const uint8_t *p = (const uint8_t *)input;
    const uint8_t *const b_end = p + len;
    uint64_t h64;

    if (len >= 32U) {
        const uint8_t *const limit = b_end - 32U;
        uint64_t v1 = seed + XXH_PRIME64_1 + XXH_PRIME64_2;
        uint64_t v2 = seed + XXH_PRIME64_2;
        uint64_t v3 = seed + 0;
        uint64_t v4 = seed - XXH_PRIME64_1;

        do {
            v1 = xxh64_round(v1, get_unaligned_le64(p + 0));
            v2 = xxh64_round(v2, get_unaligned_le64(p + 8));
            v3 = xxh64_round(v3, get_unaligned_le64(p + 16));
            v4 = xxh64_round(v4, get_unaligned_le64(p + 24));
            p += 32U;
        } while (p <= limit);

        h64 = rol64(v1, 1) + rol64(v2, 7) + rol64(v3, 12) + rol64(v4, 18);
        h64 = xxh64_merge_round(h64, v1);
        h64 = xxh64_merge_round(h64, v2);
        h64 = xxh64_merge_round(h64, v3);
        h64 = xxh64_merge_round(h64, v4);
    } else {
        h64 = seed + XXH_PRIME64_5;
    }

    h64 += (uint64_t)len;

    while (p + 8U <= b_end) {
        uint64_t k1 = xxh64_round(0, get_unaligned_le64(p));
        h64 ^= k1;
        h64 = rol64(h64, 27) * XXH_PRIME64_1 + XXH_PRIME64_4;
        p += 8U;
    }

    if (p + 4U <= b_end) {
        h64 ^= (uint64_t)get_unaligned_le32(p) * XXH_PRIME64_1;
        h64 = rol64(h64, 23) * XXH_PRIME64_2 + XXH_PRIME64_3;
        p += 4U;
    }

    while (p < b_end) {
        h64 ^= (uint64_t)(*p++) * XXH_PRIME64_5;
        h64 = rol64(h64, 11) * XXH_PRIME64_1;
    }

    return xxh64_avalanche(h64);
}

int main(void)
{
    /* Official XXH64 test vectors: empty@0 and "0123456789..." sanity checks. */
    if (kernel_xxh64("", 0, 0) != 0xEF46DB3751D8E999ULL)
        return 1;
    if (kernel_xxh64("", 0, 0x9E3779B1U) != 0xAC75FDA2929B17EFULL)
        return 2;

    uint32_t s = 0x13579BDFU;
#ifdef BENCH_DIGEST_PROBE
    /* One-bit perturbation of the input generator.
     * tests/corpus/test_benchmark_digest_sensitivity.py links every workload
     * with and without this define and requires the printed digest to move: a
     * digest blind to one changed input byte cannot catch a miscompile that
     * corrupts the data it reads (an earlier XOR-folded digest here was
     * identically zero for every input and every compiler). */
    s ^= 0x01U;
#endif
    for (size_t i = 0; i < sizeof(data_buf); i++) {
        s = rol32(s * 0x9E3779B1U, 13) + 0x85EBCA77U;
        data_buf[i] = (uint8_t)(s >> 24);
    }

    uint64_t acc = 0;
    for (unsigned int p = 0; p < PASSES; p++) {
        size_t off = (size_t)(p & 31U);
        size_t full_len = (size_t)BUF_SIZE - (size_t)((p * 13U) & 63U);
        size_t short_len = (size_t)((p * 7U) & 63U);
        acc ^= kernel_xxh64(data_buf + off, full_len, acc);
        acc += kernel_xxh64(data_buf + off + 5U, short_len, (uint64_t)p);
    }

    printf("%016llx\n", (unsigned long long)acc);
    return 0;
}
