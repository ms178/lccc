/* SPDX-License-Identifier: Zlib */
/*
 * Workload-derived kernel: zlib-ng 2.3.3, arch/generic/slide_hash_c.c +
 * insert_string_tpl.h (Zlib license).
 *
 * Exercises zlib-ng's dictionary hash insertion (`(val * 2654435761U) >> 17`)
 * paired with `slide_hash_c` (`m >= wsize ? m - wsize : 0` over 16-bit Pos
 * tables `head[HASH_SIZE]` and `prev[WSIZE]`), verifying exact table equality
 * against an unvectorized reference implementation.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef WSIZE
#define WSIZE 16384U
#endif
#ifndef HASH_BITS
#define HASH_BITS 15U
#endif
#ifndef PASSES
#define PASSES 512U
#endif

#define HASH_SIZE (1U << HASH_BITS)
typedef uint16_t Pos;

static uint8_t window_buf[2U * WSIZE + 8U];
static Pos head_tbl[HASH_SIZE];
static Pos prev_tbl[WSIZE];
static Pos ref_head[HASH_SIZE];
static Pos ref_prev[WSIZE];

static inline uint32_t load_u32_le(const uint8_t *p)
{
    uint32_t v;
    memcpy(&v, p, sizeof(v));
    return v;
}

__attribute__((noinline))
static Pos zng_insert_string_batch(const uint8_t *window, Pos *headp, Pos *prevp,
                                   uint32_t str, uint32_t count, uint32_t w_mask)
{
    const uint8_t *strstart = window + str;
    Pos idx = (Pos)str;
    Pos match_head = 0;

    for (uint32_t n = count; n != 0U; n--, idx++, strstart++) {
        uint32_t val = load_u32_le(strstart);
        uint32_t hm = (val * 2654435761U) >> (32U - HASH_BITS);
        Pos h = headp[hm];
        if (h != idx) {
            prevp[idx & w_mask] = h;
            headp[hm] = idx;
        }
        match_head = h;
    }
    return match_head;
}

static inline void slide_hash_c_chain(Pos *table, uint32_t entries, uint16_t wsize)
{
    Pos *q = table;
    for (uint32_t i = 0; i < entries; i++) {
        Pos m = *q;
        Pos t = (Pos)wsize;
        *q++ = (Pos)(m >= t ? (Pos)(m - t) : 0U);
    }
}

__attribute__((noinline))
static void zng_slide_hash_c(Pos *headp, Pos *prevp, uint16_t wsize)
{
    slide_hash_c_chain(headp, HASH_SIZE, wsize);
    slide_hash_c_chain(prevp, wsize, wsize);
}

static void ref_slide_chain(Pos *table, uint32_t entries, uint16_t wsize)
{
    for (uint32_t i = 0; i < entries; i++) {
        uint32_t v = table[i];
        table[i] = (v >= (uint32_t)wsize) ? (Pos)(v - wsize) : 0U;
        __asm__ volatile("" : "+m"(table[i]));
    }
}

int main(void)
{
    uint32_t s = 0x243F6A88U;
    for (size_t i = 0; i < sizeof(window_buf); i++) {
        s = s * 1664525U + 1013904223U;
        window_buf[i] = (uint8_t)((s >> 20) ^ (s >> 11));
    }

    memset(head_tbl, 0, sizeof(head_tbl));
    memset(prev_tbl, 0, sizeof(prev_tbl));
    (void)zng_insert_string_batch(window_buf, head_tbl, prev_tbl,
                                  1U, 2U * WSIZE - 8U, WSIZE - 1U);
    memcpy(ref_head, head_tbl, sizeof(head_tbl));
    memcpy(ref_prev, prev_tbl, sizeof(prev_tbl));

    zng_slide_hash_c(head_tbl, prev_tbl, (uint16_t)WSIZE);
    ref_slide_chain(ref_head, HASH_SIZE, (uint16_t)WSIZE);
    ref_slide_chain(ref_prev, WSIZE, (uint16_t)WSIZE);

    if (memcmp(head_tbl, ref_head, sizeof(head_tbl)) != 0 ||
        memcmp(prev_tbl, ref_prev, sizeof(prev_tbl)) != 0)
        return 1;

    uint64_t acc = 0;
    for (uint32_t p = 0; p < PASSES; p++) {
        uint32_t base = (p * 257U) & (WSIZE - 1U);
        Pos mh = zng_insert_string_batch(window_buf, head_tbl, prev_tbl,
                                         WSIZE + (base & 511U), 512U, WSIZE - 1U);
        zng_slide_hash_c(head_tbl, prev_tbl, (uint16_t)(WSIZE >> 1));
        acc = (acc * 1315423911ULL) ^ (uint64_t)mh ^ head_tbl[p & (HASH_SIZE - 1U)];
    }

    for (uint32_t i = 0; i < HASH_SIZE; i += 17U)
        acc ^= ((uint64_t)head_tbl[i] << 16) | prev_tbl[i & (WSIZE - 1U)];

    printf("%016llx\n", (unsigned long long)acc);
    return 0;
}
