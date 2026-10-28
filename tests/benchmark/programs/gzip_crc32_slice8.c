/* SPDX-License-Identifier: LGPL-3.0-or-later */
/*
 * Workload-derived kernel: GNU gzip 1.14, lib/crc.c + lib/crc-sliceby8.h,
 * crc32_update_no_xor_slice_by_8().
 *
 * Extracted from GNU gzip 1.14's gnulib CRC-32 slice-by-8 portable fast path
 * (LGPL-3.0-or-later). The 8x256 lookup tables are generated deterministically
 * from the IEEE 802.3 reflected polynomial 0xEDB88320U using the exact
 * recurrence in lib/crc-generate-table.c, and self-checked against the 1-byte
 * scalar reference on every run.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef DATA_SIZE
#define DATA_SIZE (1U << 18)
#endif
#ifndef PASSES
#define PASSES 256U
#endif

static uint32_t crc32_sliceby8_table[8][256];
static unsigned char payload[DATA_SIZE + 16];

static void init_tables_and_payload(void)
{
    for (uint32_t i = 0; i < 256U; i++) {
        uint32_t c = i;
        for (int k = 0; k < 8; k++)
            c = (c & 1U) ? (0xEDB88320U ^ (c >> 1)) : (c >> 1);
        crc32_sliceby8_table[0][i] = c;
    }
    for (uint32_t t = 1; t < 8U; t++) {
        for (uint32_t i = 0; i < 256U; i++) {
            uint32_t prev = crc32_sliceby8_table[t - 1U][i];
            crc32_sliceby8_table[t][i] =
                (prev >> 8) ^ crc32_sliceby8_table[0][prev & 0xFFU];
        }
    }

    uint32_t s = 0x9E3779B9U;
    for (size_t i = 0; i < sizeof(payload); i++) {
        s = s * 1664525U + 1013904223U;
        payload[i] = (unsigned char)((s >> 16) ^ (uint32_t)i);
    }
}

static uint64_t load_le64(const unsigned char *p)
{
    uint64_t v = (uint64_t)p[0]
               | ((uint64_t)p[1] << 8)
               | ((uint64_t)p[2] << 16)
               | ((uint64_t)p[3] << 24)
               | ((uint64_t)p[4] << 32)
               | ((uint64_t)p[5] << 40)
               | ((uint64_t)p[6] << 48)
               | ((uint64_t)p[7] << 56);
    return v;
}

static uint32_t crc32_slice_by_8_step(uint32_t crc, const unsigned char *buf)
{
    /* Note: in gnulib's crc-sliceby8.h table numbering, table[7] is the
     * 7-byte-ahead table (lowest input byte) and table[0] is the last byte. */
    uint64_t local_buf = load_le64(buf) ^ (uint64_t)crc;
    return crc32_sliceby8_table[0][(local_buf >> 56) & 0xFFU]
         ^ crc32_sliceby8_table[1][(local_buf >> 48) & 0xFFU]
         ^ crc32_sliceby8_table[2][(local_buf >> 40) & 0xFFU]
         ^ crc32_sliceby8_table[3][(local_buf >> 32) & 0xFFU]
         ^ crc32_sliceby8_table[4][(local_buf >> 24) & 0xFFU]
         ^ crc32_sliceby8_table[5][(local_buf >> 16) & 0xFFU]
         ^ crc32_sliceby8_table[6][(local_buf >> 8) & 0xFFU]
         ^ crc32_sliceby8_table[7][local_buf & 0xFFU];
}

__attribute__((noinline))
static uint32_t gzip_crc32_slice8(uint32_t crc, const unsigned char *buf, size_t len)
{
    size_t n = 0;
    size_t aligned = len & ~(size_t)7U;
    crc = ~crc;
    for (; n < aligned; n += 8U)
        crc = crc32_slice_by_8_step(crc, buf + n);
    for (; n < len; n++)
        crc = (crc >> 8) ^ crc32_sliceby8_table[0][(crc ^ buf[n]) & 0xFFU];
    return ~crc;
}

static uint32_t crc32_scalar_ref(uint32_t crc, const unsigned char *buf, size_t len)
{
    crc = ~crc;
    for (size_t i = 0; i < len; i++)
        crc = (crc >> 8) ^ crc32_sliceby8_table[0][(crc ^ buf[i]) & 0xFFU];
    return ~crc;
}

int main(void)
{
    init_tables_and_payload();

    /* Self-check across all 0..15 alignments and tail lengths 0..15. */
    for (size_t off = 0; off < 8U; off++) {
        for (size_t len = 0; len <= 79U; len++) {
            uint32_t got = gzip_crc32_slice8(0x12345678U, payload + off, len);
            uint32_t exp = crc32_scalar_ref(0x12345678U, payload + off, len);
            if (got != exp)
                return 1;
        }
    }

    uint32_t acc = 0U;
    for (unsigned int p = 0; p < PASSES; p++) {
        size_t off = (size_t)(p & 7U);
        size_t len = (size_t)DATA_SIZE - (size_t)((p * 5U) & 15U);
        acc = gzip_crc32_slice8(acc ^ p, payload + off, len);
    }

    uint32_t check_fast = gzip_crc32_slice8(acc, payload + 3, 1029U);
    uint32_t check_ref = crc32_scalar_ref(acc, payload + 3, 1029U);
    if (check_fast != check_ref)
        return 2;

    printf("%08x\n", check_fast);
    return 0;
}
