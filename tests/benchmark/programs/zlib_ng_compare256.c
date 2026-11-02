/* SPDX-License-Identifier: Zlib */
/*
 * zlib-ng 2.3.3 arch/generic/compare256_p.h: compare256_64().
 * Copyright (C) 2020 Nathan Moinvaziri; distribution under the zlib license.
 *
 * This benchmark retains the upstream 64-bit XOR / count-leading-or-trailing-
 * zeros comparison, specialized to the 256-byte match length. zng_memread_8
 * is expressed by its upstream memcpy fallback so unaligned reads and C's
 * aliasing rules are respected. The separate input generator and exhaustive
 * reference checks below are not part of the upstream project. It is a kernel,
 * not a substitute for a complete compression/decompression workload.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef PASSES
#define PASSES 8192U
#endif
#if PASSES < 1
#error PASSES must be positive
#endif

#define CASE_COUNT 64U
#define MATCH_LENGTH 256U
#define SLACK 8U

static unsigned char left[CASE_COUNT][MATCH_LENGTH + SLACK];
static unsigned char right[CASE_COUNT][MATCH_LENGTH + SLACK];
static unsigned int mismatch[CASE_COUNT];

/* Keep the measured body visible for assembly comparison and prevent a
 * caller from turning fixed input matches into compile-time constants. */
__attribute__((noinline))
uint32_t zlib_compare256_64(const unsigned char *src0, const unsigned char *src1)
{
    uint32_t len = 0;
    do {
        uint64_t sv, mv, diff;
        memcpy(&sv, src0, sizeof(sv));
        memcpy(&mv, src1, sizeof(mv));
        diff = sv ^ mv;
        if (diff) {
#if __BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__
            uint64_t match_byte = (uint64_t)__builtin_ctzll(diff) / 8;
#else
            uint64_t match_byte = (uint64_t)__builtin_clzll(diff) / 8;
#endif
            return len + (uint32_t)match_byte;
        }
        src0 += 8;
        src1 += 8;
        len += 8;
    } while (len < MATCH_LENGTH);
    return MATCH_LENGTH;
}

static uint32_t reference(const unsigned char *a, const unsigned char *b)
{
    uint32_t n;
    for (n = 0; n < MATCH_LENGTH && a[n] == b[n]; n++)
        ;
    return n;
}

static int check_all_offsets(void)
{
    unsigned char a[MATCH_LENGTH + SLACK];
    unsigned char b[MATCH_LENGTH + SLACK];
    unsigned int i, left_align, right_align, pos;
    for (i = 0; i < sizeof(a); i++)
        a[i] = (unsigned char)(i * 29U + (i >> 2) + 17U);
    for (left_align = 0; left_align < SLACK; left_align++) {
        for (right_align = 0; right_align < SLACK; right_align++) {
            for (pos = 0; pos <= MATCH_LENGTH; pos++) {
                memcpy(b + right_align, a + left_align, MATCH_LENGTH);
                if (pos < MATCH_LENGTH)
                    b[right_align + pos] ^= 0x80U;
                if (reference(a + left_align, b + right_align) != pos
                    || zlib_compare256_64(a + left_align, b + right_align) != pos)
                    return 0;
            }
        }
        if (zlib_compare256_64(a + left_align, a + left_align) != MATCH_LENGTH)
            return 0;
    }
    return 1;
}

static void make_inputs(void)
{
    unsigned int c, i;
    for (c = 0; c < CASE_COUNT; c++) {
        unsigned int left_align = c % SLACK;
        unsigned int right_align = (c / SLACK) % SLACK;
        mismatch[c] = c == CASE_COUNT - 1 ? MATCH_LENGTH : (c * 73U) % MATCH_LENGTH;
        for (i = 0; i < MATCH_LENGTH + SLACK; i++)
            left[c][i] = (unsigned char)(i * 17U + c * 31U + (i >> 3));
        memcpy(right[c] + right_align, left[c] + left_align, MATCH_LENGTH);
        if (mismatch[c] < MATCH_LENGTH)
            right[c][right_align + mismatch[c]] ^= 0x40U;
    }
}

int main(void)
{
    uint64_t checksum = 1469598103934665603ULL;
    unsigned int c, pass;
    if (!check_all_offsets())
        return 2;
    make_inputs();
    for (c = 0; c < CASE_COUNT; c++) {
        unsigned int left_align = c % SLACK;
        unsigned int right_align = (c / SLACK) % SLACK;
        if (reference(left[c] + left_align, right[c] + right_align) != mismatch[c]
            || zlib_compare256_64(left[c] + left_align, right[c] + right_align) != mismatch[c])
            return 3;
    }
    for (pass = 0; pass < PASSES; pass++) {
        for (c = 0; c < CASE_COUNT; c++) {
            unsigned int n = (c + pass) % CASE_COUNT;
            unsigned int left_align = n % SLACK;
            unsigned int right_align = (n / SLACK) % SLACK;
            uint32_t len = zlib_compare256_64(left[n] + left_align, right[n] + right_align);
            checksum = (checksum ^ (uint64_t)(len + n + 1U)) * 1099511628211ULL;
        }
    }
    printf("zlib_ng_compare256 checksum=%llu\n", (unsigned long long)checksum);
    return 0;
}
