/* SPDX-License-Identifier: LicenseRef-SQLite-Public-Domain */
/*
 * Workload-derived kernel: SQLite 3.54.0 (sqlite-src-3540000), src/vdbeaux.c,
 * sqlite3VdbeSerialTypeLen() + sqlite3VdbeSerialGet().
 *
 * Decodes packed SQLite B-tree record headers and column payloads across all
 * SQLite serial types (NULL, 8/16/24/32/48/64-bit big-endian twos-complement
 * integers, IEEE-754 float64, constants 0/1, and variable-length blobs/text),
 * verifying the decoded integer/length sums against the exact values encoded
 * when constructing the test records.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef RECORD_COUNT
#define RECORD_COUNT 2048U
#endif
#ifndef PASSES
#define PASSES 256U
#endif

static const uint8_t sqlite3SmallTypeSizes[128] = {
    /* 0..11 */ 0, 1, 2, 3, 4, 6, 8, 8, 0, 0, 0, 0,
    /* 12..27 */ 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7,
    /* 28..43 */ 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13, 14, 14, 15, 15,
    /* 44..59 */ 16, 16, 17, 17, 18, 18, 19, 19, 20, 20, 21, 21, 22, 22, 23, 23,
    /* 60..75 */ 24, 24, 25, 25, 26, 26, 27, 27, 28, 28, 29, 29, 30, 30, 31, 31,
    /* 76..91 */ 32, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37, 38, 38, 39, 39,
    /* 92..107 */ 40, 40, 41, 41, 42, 42, 43, 43, 44, 44, 45, 45, 46, 46, 47, 47,
    /* 108..127 */ 48, 48, 49, 49, 50, 50, 51, 51, 52, 52, 53, 53, 54, 54, 55, 55,
    56, 56, 57, 57
};

enum {
    MEM_Null = 0x01,
    MEM_Int  = 0x02,
    MEM_Real = 0x04,
    MEM_Str  = 0x08,
    MEM_Blob = 0x10
};

typedef struct {
    int64_t i;
    double r;
    const uint8_t *z;
    uint32_t n;
    uint16_t flags;
} VdbeCell;

static uint8_t arena[RECORD_COUNT * 64U];
static uint32_t rec_offsets[RECORD_COUNT];

static inline uint32_t serial_type_len(uint32_t serial_type)
{
    if (serial_type >= 128U)
        return (serial_type - 12U) >> 1;
    return sqlite3SmallTypeSizes[serial_type];
}

static inline uint32_t get_u32_be(const uint8_t *p)
{
    return ((uint32_t)p[0] << 24)
         | ((uint32_t)p[1] << 16)
         | ((uint32_t)p[2] << 8)
         | (uint32_t)p[3];
}

static inline uint64_t get_u64_be(const uint8_t *p)
{
    return ((uint64_t)get_u32_be(p) << 32) | (uint64_t)get_u32_be(p + 4);
}

__attribute__((noinline))
static void vdbe_serial_get(const uint8_t *buf, uint32_t serial_type, VdbeCell *cell)
{
    switch (serial_type) {
    case 0:
    case 10:
    case 11:
        cell->flags = MEM_Null;
        cell->i = 0;
        return;
    case 1:
        cell->i = (int8_t)buf[0];
        cell->flags = MEM_Int;
        return;
    case 2:
        cell->i = (int16_t)(((uint16_t)buf[0] << 8) | (uint16_t)buf[1]);
        cell->flags = MEM_Int;
        return;
    case 3: {
        uint32_t u = ((uint32_t)(int32_t)(int8_t)buf[0] << 16)
                   | ((uint32_t)buf[1] << 8)
                   | (uint32_t)buf[2];
        cell->i = (int32_t)u;
        cell->flags = MEM_Int;
        return;
    }
    case 4:
        cell->i = (int32_t)get_u32_be(buf);
        cell->flags = MEM_Int;
        return;
    case 5: {
        int64_t hi = (int16_t)(((uint16_t)buf[0] << 8) | (uint16_t)buf[1]);
        uint64_t lo = get_u32_be(buf + 2);
        cell->i = (int64_t)(((uint64_t)hi << 32) | lo);
        cell->flags = MEM_Int;
        return;
    }
    case 6:
        cell->i = (int64_t)get_u64_be(buf);
        cell->flags = MEM_Int;
        return;
    case 7: {
        uint64_t raw = get_u64_be(buf);
        memcpy(&cell->r, &raw, sizeof(raw));
        cell->flags = MEM_Real;
        return;
    }
    case 8:
        cell->i = 0;
        cell->flags = MEM_Int;
        return;
    case 9:
        cell->i = 1;
        cell->flags = MEM_Int;
        return;
    default:
        cell->z = buf;
        cell->n = serial_type_len(serial_type);
        cell->flags = (serial_type & 1U) ? MEM_Str : MEM_Blob;
        return;
    }
}

static void put_be_n(uint8_t *dst, uint64_t v, int n)
{
    for (int i = n - 1; i >= 0; i--) {
        dst[i] = (uint8_t)(v & 0xFFU);
        v >>= 8;
    }
}

static int64_t build_records(void)
{
    uint32_t pos = 0;
    uint32_t s = 0xA5A5F00DU;
    int64_t expected_sum = 0;

    for (uint32_t r = 0; r < RECORD_COUNT; r++) {
        rec_offsets[r] = pos;
        uint8_t *hdr = arena + pos;
        hdr[0] = 9; /* header length: 1 + 8 single-byte serial types */
        uint8_t *body = hdr + 9;
        for (uint32_t col = 0; col < 8U; col++) {
            s = s * 1664525U + 1013904223U;
            uint32_t kind = (r + col) % 10U;
            if (kind == 0U) {
                hdr[1U + col] = 0;
            } else if (kind == 1U) {
                int8_t v = (int8_t)(s >> 24);
                hdr[1U + col] = 1;
                body[0] = (uint8_t)v;
                body += 1;
                expected_sum += v;
            } else if (kind == 2U) {
                int16_t v = (int16_t)(s >> 16);
                hdr[1U + col] = 2;
                put_be_n(body, (uint16_t)v, 2);
                body += 2;
                expected_sum += v;
            } else if (kind == 3U) {
                int32_t v = ((int32_t)s >> 8);
                hdr[1U + col] = 3;
                put_be_n(body, (uint32_t)v & 0xFFFFFFU, 3);
                body += 3;
                expected_sum += v;
            } else if (kind == 4U) {
                int32_t v = (int32_t)s;
                hdr[1U + col] = 4;
                put_be_n(body, (uint32_t)v, 4);
                body += 4;
                expected_sum += v;
            } else if (kind == 5U) {
                int64_t v = (int64_t)(((uint64_t)(int64_t)(int32_t)s << 16) ^ (uint64_t)r);
                hdr[1U + col] = 5;
                put_be_n(body, (uint64_t)v & 0xFFFFFFFFFFFFULL, 6);
                body += 6;
                expected_sum += v;
            } else if (kind == 6U) {
                int64_t v = ((int64_t)(int32_t)s * 1000003LL) - (int64_t)col;
                hdr[1U + col] = 6;
                put_be_n(body, (uint64_t)v, 8);
                body += 8;
                expected_sum += v;
            } else if (kind == 7U) {
                hdr[1U + col] = 8;
            } else if (kind == 8U) {
                hdr[1U + col] = 9;
                expected_sum += 1;
            } else {
                uint32_t len = (s & 7U) + 1U;
                hdr[1U + col] = (uint8_t)(13U + 2U * len);
                for (uint32_t k = 0; k < len; k++)
                    body[k] = (uint8_t)('A' + ((s + k) & 15U));
                body += len;
                expected_sum += (int64_t)len;
            }
        }
        pos = (uint32_t)(body - arena);
    }
    return expected_sum;
}

__attribute__((noinline))
static int64_t scan_all_records(void)
{
    uint64_t total = 0;
    VdbeCell cell;
    for (uint32_t r = 0; r < RECORD_COUNT; r++) {
        const uint8_t *rec = arena + rec_offsets[r];
        uint32_t hdr_len = rec[0];
        const uint8_t *body = rec + hdr_len;
        for (uint32_t h = 1; h < hdr_len; h++) {
            uint32_t st = rec[h];
            vdbe_serial_get(body, st, &cell);
            body += serial_type_len(st);
            if (cell.flags == MEM_Int)
                total += (uint64_t)cell.i;
            else if (cell.flags & (MEM_Str | MEM_Blob))
                total += (uint64_t)cell.n;
        }
    }
    return (int64_t)total;
}

int main(void)
{
    int64_t expected = build_records();
    int64_t single = scan_all_records();
    if (single != expected)
        return 1;

    uint64_t acc = 0;
    for (uint32_t p = 0; p < PASSES; p++)
        acc += (uint64_t)scan_all_records() ^ (uint64_t)p;

    printf("%016llx\n", (unsigned long long)acc);
    return 0;
}
