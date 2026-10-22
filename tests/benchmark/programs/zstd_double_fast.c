/*
 * Workload-derived kernel: Zstandard 1.5.7, lib/compress/zstd_double_fast.c.
 *
 * The double-hash match search below is extracted from Zstandard's
 * zstd_double_fast.c (BSD-3-Clause OR GPL-2.0). ZSTD's context, block
 * iteration, sequence encoding and entropy stages were removed; the harness
 * builds a deterministic input, runs the same two-table hash search the
 * compression loop performs, and checks the matches it produces against a naive
 * scan. Names are prefixed. Full source provenance is in
 * tests/benchmark/WORKLOAD_PROVENANCE.md.
 *
 * Why this shape: ZSTD_compressBlock_doubleFast is Zstandard's *fast* level
 * matcher and it is the shape a code generator most often gets wrong, because
 * the whole inner loop is built out of unaligned 32/64-bit loads whose results
 * feed address arithmetic:
 *
 *   * `MEM_read32` / `MEM_read64` at arbitrary offsets -- the kernel keeps both
 *     as plain loads and relies on the target allowing unaligned access, so a
 *     backend that conservatively splits them into byte loads loses most of
 *     the throughput;
 *   * two independent hash tables (`hashLong` 8-byte and `hashSmall` 4-byte)
 *     probed in the same iteration -- two dependent load chains that a
 *     scheduler should overlap;
 *   * `ZSTD_count` extension of a candidate match using a 64-bit XOR plus
 *     `__builtin_ctzll`, then an unaligned 64-bit compare loop.
 *
 * The self-check compares every reported match length against a byte-by-byte
 * scan of the same candidate, so the kernel cannot silently "win" by finding
 * shorter matches.
 */
#include <stdio.h>
#include <string.h>

#ifndef INPUT_SIZE
#define INPUT_SIZE (1UL << 16)
#endif
#ifndef PASSES
#define PASSES 64U
#endif
#ifndef MIN_MATCH
#define MIN_MATCH 4
#endif

typedef unsigned int u32;
typedef unsigned long long u64;

#define HASHLOG_SMALL 10
#define HASHLOG_LONG 12
#define HASH_SMALL_SIZE (1U << HASHLOG_SMALL)
#define HASH_LONG_SIZE (1U << HASHLOG_LONG)
#define MAX_DISTANCE (1U << 16)

static unsigned char input[INPUT_SIZE + 64];
static u32 hash_small[HASH_SMALL_SIZE];
static u32 hash_long[HASH_LONG_SIZE];

/* ZSTD's MEM_read32 / MEM_read64: deliberately unaligned. */
static u32 read32(const void *p)
{
    u32 v;
    memcpy(&v, p, sizeof v);
    return v;
}

static u64 read64(const void *p)
{
    u64 v;
    memcpy(&v, p, sizeof v);
    return v;
}

/* ZSTD_XXH64-style multiplicative hash truncation, as used by
 * ZSTD_hashPtr for the 4- and 8-byte tables. Only the top bits are kept. */
static u32 hash4(u32 sequence)
{
    return (u32)((sequence * 2654435761U) >> (32 - HASHLOG_SMALL));
}

static u32 hash8(u64 sequence)
{
    return (u32)((sequence * 11400714819323198485ULL) >> (64 - HASHLOG_LONG));
}

/* ZSTD_count: extend a match with 64-bit XOR + ctz, then bytes. */
static u32 count_match(const unsigned char *p_in, const unsigned char *p_match,
                       const unsigned char *p_in_limit)
{
    const unsigned char *start = p_in;
    while (p_in + 8 <= p_in_limit) {
        u64 diff = read64(p_in) ^ read64(p_match);
        if (diff != 0)
            return (u32)(p_in - start) + ((u32)__builtin_ctzll(diff) >> 3);
        p_in += 8;
        p_match += 8;
    }
    while (p_in < p_in_limit && *p_in == *p_match) {
        p_in++;
        p_match++;
    }
    return (u32)(p_in - start);
}

/* One pass of the double-fast search: exactly the two-table probe of
 * ZSTD_compressBlock_doubleFast, with the block walk reduced to a step loop. */
static u64 search_pass(u32 start, u32 end, int *mismatch)
{
    const unsigned char *base = input;
    const unsigned char *ip = base + start;
    const unsigned char *iend = base + end;
    const unsigned char *ilimit = iend - 8;
    u64 total = 0;

    while (ip < ilimit) {
        u32 small = read32(ip);
        u32 h_small = hash4(small);
        u32 h_long = hash8(read64(ip));
        u32 match_index_small = hash_small[h_small];
        u32 match_index_long = hash_long[h_long];

        hash_small[h_small] = (u32)(ip - base);
        hash_long[h_long] = (u32)(ip - base);

        /* Long-table candidate first, exactly as double-fast does. */
        if (match_index_long < (u32)(ip - base)
            && (u32)(ip - base) - match_index_long <= MAX_DISTANCE
            && read64(base + match_index_long) == read64(ip)) {
            const unsigned char *match = base + match_index_long;
            u32 len = MIN_MATCH + count_match(ip + MIN_MATCH, match + MIN_MATCH, iend);
            /* Self-check: the same length from a byte-wise scan. */
            u32 reference = 0;
            while (ip + reference < iend && match[reference] == ip[reference])
                reference++;
            if (reference != len)
                *mismatch = 1;
            total += len;
            ip += len > 0 ? len : 1;
            continue;
        }
        if (match_index_small < (u32)(ip - base)
            && (u32)(ip - base) - match_index_small <= MAX_DISTANCE
            && read32(base + match_index_small) == small) {
            const unsigned char *match = base + match_index_small;
            u32 len = MIN_MATCH + count_match(ip + MIN_MATCH, match + MIN_MATCH, iend);
            u32 reference = 0;
            while (ip + reference < iend && match[reference] == ip[reference])
                reference++;
            if (reference != len)
                *mismatch = 1;
            total += len;
            ip += len > 0 ? len : 1;
            continue;
        }
        ip += 1;
    }
    return total;
}

int main(void)
{
    u64 checksum = 0;
    unsigned int pass, i;
    int mismatch = 0;

    for (i = 0; i < INPUT_SIZE + 64; i++) {
        u32 state = i * 2654435761U + 12345U;
        input[i] = (unsigned char)('A' + ((state >> 13) % 7));
        if (i > 64 && (state & 3) == 0)
            input[i] = input[i - 1 - ((state >> 7) & 31)];
        if (i > 1024 && (state & 7) == 0)
            input[i] = input[i - 1024 + ((state >> 9) & 7)];
    }

    for (pass = 0; pass < PASSES; pass++) {
        u64 total;
        /* The tables are rebuilt per pass: a pass must not depend on the
         * previous pass's table state for its own result to be meaningful. */
        memset(hash_small, 0, sizeof hash_small);
        memset(hash_long, 0, sizeof hash_long);
        total = search_pass(16, (u32)(INPUT_SIZE - 16), &mismatch);
        checksum = checksum * 1000003ULL + total;
    }

    printf("zstd_double_fast checksum=%llu matches_verified=%s\n",
           checksum, mismatch ? "MISMATCH" : "OK");
    return mismatch ? 1 : 0;
}
