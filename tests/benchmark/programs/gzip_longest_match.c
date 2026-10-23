/*
 * Workload-derived kernel: GNU gzip 1.14, deflate.c, longest_match().
 *
 * The match-finder loop below is extracted from GNU gzip's deflate.c
 * (LGPL-3.0-or-later). Configuration macros, the sliding-window/output
 * machinery, zlib's gz* interfaces and the hash-table *insertion* policy were
 * removed; the hash chain is built once by this harness and then searched
 * exactly as longest_match() does. Names are prefixed. Full source provenance
 * is in tests/benchmark/WORKLOAD_PROVENANCE.md.
 *
 * Why this shape: longest_match is the single hottest function of gzip's
 * compression path, and it is a stress test for precisely the things a code
 * generator controls --
 *
 *   * an unaligned multi-byte load pair compared as one word
 *     (gzip's `#if defined UNALIGNED_OK` arm: two `ush`-style loads widened to
 *     an unsigned word), then a trailing-zero count to find the first
 *     differing byte;
 *   * a pointer-chasing chain walk whose trip count is data dependent
 *     (the `--chain_length` bound), so the loop is branch-heavy and
 *     unpredictable;
 *   * a loop-carried best-length/best-match pair that must stay in registers
 *     across the inner compare to avoid a spill in the loop body.
 *
 * The `quick_bytes` pre-check is also kept: gzip skips a candidate whose byte
 * at the current best length already differs, which is the branch that makes
 * the whole loop cheap on real data.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef WINDOW_SIZE
#define WINDOW_SIZE (1UL << 16)
#endif
#ifndef CHAIN_LENGTH
#define CHAIN_LENGTH 128
#endif
#ifndef MIN_MATCH
#define MIN_MATCH 3
#endif
#ifndef MAX_MATCH
#define MAX_MATCH 258
#endif
#ifndef PASSES
/* Keep default timing runs above the process-startup noise floor.
 * Bounded Callgrind screens explicitly override PASSES=1. */
#define PASSES 8192U
#endif
#ifndef HASH_BITS
#define HASH_BITS 15
#endif

#define HASH_SIZE (1U << HASH_BITS)
#define NIL 0xFFFFU

static unsigned char window[WINDOW_SIZE + MAX_MATCH];
static unsigned int head[HASH_SIZE];
static unsigned int prev[WINDOW_SIZE];
/* Search positions used by every pass: a deterministic, repeatable schedule so
 * the measured work does not depend on the random-number implementation of the
 * host's libc. */
static unsigned int positions[256];
static unsigned int position_count;

static unsigned int hash3(const unsigned char *p)
{
    unsigned int h = (unsigned int)p[0] << 10;
    h ^= (unsigned int)p[1] << 5;
    h ^= (unsigned int)p[2];
    return h & (HASH_SIZE - 1);
}

/* Build gzip's hash chain over the window, exactly as INSERT_STRING does. */
static void build_chain(void)
{
    unsigned int i;
    for (i = 0; i < HASH_SIZE; i++)
        head[i] = NIL;
    for (i = 0; i < WINDOW_SIZE; i++)
        prev[i] = NIL;
    for (i = 0; i + MIN_MATCH <= WINDOW_SIZE; i++) {
        unsigned int h = hash3(window + i);
        prev[i] = head[h];
        head[h] = i;
    }
}

/* Read MIN_MATCH bytes as one integer, the portable spelling of gzip's
 * UNALIGNED_OK arm. memcpy keeps the load valid at any alignment; every
 * compiler with an unaligned 32-bit load folds it. */
static unsigned int load32(const unsigned char *p)
{
    unsigned int v;
    memcpy(&v, p, sizeof v);
    return v;
}

/* MIN_MATCH bytes, zero-extended. The pre-check must compare exactly MIN_MATCH
 * bytes: comparing four rejects a candidate whose first three bytes agree and
 * whose fourth differs, which is still a match of length 3 -- the reference
 * below finds it, and gzip's own `scan_start` holds exactly two bytes. */
static unsigned int load24(const unsigned char *p)
{
    return load32(p) & 0x00FFFFFFU;
}

static unsigned int trailing_zero_bytes(unsigned int diff)
{
    /* gzip compares the widened load little-endian, so the first differing
     * byte is the lowest non-zero byte: a count of trailing zeros. */
    return (unsigned int)__builtin_ctz(diff) >> 3;
}

/* gzip longest_match(), with the window offset that gzip's cur_match/strstart
 * arithmetic performs already resolved into absolute indices. */
static unsigned int longest_match(unsigned int cur_match, unsigned int strstart,
                                  unsigned int prev_length)
{
    unsigned int chain_length = CHAIN_LENGTH;
    unsigned int best_len = prev_length;
    unsigned int best_match = NIL;
    unsigned int limit;
    const unsigned char *scan = window + strstart;
    const unsigned char *scan_end = window + strstart + best_len;
    unsigned int max_len = MAX_MATCH;
    unsigned int window_room = WINDOW_SIZE - strstart;

    if (max_len > window_room)
        max_len = window_room;
    if (best_len >= max_len)
        return best_len;

    /* gzip: "Do not waste too much time if we already have a good match." */
    if (best_len >= 32)
        chain_length >>= 2;
    limit = (strstart > MAX_MATCH ? strstart - MAX_MATCH : 0);

    for (;;) {
        const unsigned char *match = window + cur_match;
        /* quick check: the byte at the current best length must match */
        if (match[best_len] != *scan_end || match[best_len - 1] != scan[best_len - 1]
            || load24(match) != load24(scan)) {
            unsigned int pc = prev[cur_match];
            if (pc == NIL || pc < limit || --chain_length == 0)
                break;
            cur_match = pc;
            continue;
        }

        /* The candidate survives the pre-checks: extend it. gzip compares the
         * first MIN_MATCH bytes via the load above and then walks bytes. This
         * kernel keeps gzip's byte loop for the tail and uses the word compare
         * for the body, which is what the UNALIGNED_OK configuration does. */
        {
            unsigned int len = MIN_MATCH;
            while (len + 4 <= max_len) {
                unsigned int diff = load32(match + len) ^ load32(scan + len);
                if (diff != 0) {
                    len += trailing_zero_bytes(diff);
                    break;
                }
                len += 4;
            }
            while (len < max_len && match[len] == scan[len])
                len++;
            if (len > best_len) {
                best_len = len;
                best_match = cur_match;
                scan_end = window + strstart + best_len;
                if (len >= max_len)
                    break;
                if (len >= 32)
                    chain_length >>= 2;
            }
        }

        {
            unsigned int pc = prev[cur_match];
            if (pc == NIL || pc < limit || --chain_length == 0)
                break;
            cur_match = pc;
        }
    }
    (void)best_match;
    return best_len;
}

/* A plainly written reference: the naive byte-by-byte longest match over the
 * same chain. Used only to prove the fast path is not inventing matches, so
 * the harness has an algorithmic self-check that does not depend on a golden
 * number produced by any one compiler. */
static unsigned int longest_match_reference(unsigned int cur_match, unsigned int strstart,
                                            unsigned int prev_length)
{
    unsigned int chain_length = CHAIN_LENGTH;
    unsigned int best_len = prev_length;
    unsigned int limit = (strstart > MAX_MATCH ? strstart - MAX_MATCH : 0);
    unsigned int max_len = MAX_MATCH;
    const unsigned char *scan = window + strstart;
    unsigned int window_room = WINDOW_SIZE - strstart;

    if (max_len > window_room)
        max_len = window_room;
    if (best_len >= max_len)
        return best_len;
    if (best_len >= 32)
        chain_length >>= 2;

    for (;;) {
        const unsigned char *match = window + cur_match;
        unsigned int len = 0;
        while (len < max_len && match[len] == scan[len])
            len++;
        if (len > best_len) {
            best_len = len;
            if (len >= max_len)
                break;
            if (len >= 32)
                chain_length >>= 2;
        }
        {
            unsigned int pc = prev[cur_match];
            if (pc == NIL || pc < limit || --chain_length == 0)
                break;
            cur_match = pc;
        }
    }
    return best_len;
}

static unsigned long long mix(unsigned long long h, unsigned long long v)
{
    h ^= v + 0x9E3779B97F4A7C15ULL + (h << 6) + (h >> 2);
    return h;
}

int main(void)
{
    unsigned long long checksum = 0;
    unsigned long long reference = 0;
    unsigned int pass, i;

    /* Deterministic, compressible-but-not-trivial window: a small alphabet with
     * repeated runs, so matches actually occur. */
    {
        unsigned int state = 0x12345678U;
        for (i = 0; i < WINDOW_SIZE + MAX_MATCH; i++) {
            state = state * 1103515245U + 12345U;
            window[i] = (unsigned char)('a' + ((state >> 16) % 5));
            if (i > 40 && (state & 7) == 0)
                window[i] = window[i - 1 - ((state >> 8) & 15)];
            if (i > 300 && (state & 15) == 0)
                window[i] = window[i - 257];
        }
    }
    build_chain();

    position_count = 0;
    for (i = 512; i < WINDOW_SIZE - MAX_MATCH && position_count < 256; i += 61)
        positions[position_count++] = i;

    for (pass = 0; pass < PASSES; pass++)
        for (i = 0; i < position_count; i++) {
            unsigned int strstart = positions[i];
            /* gzip inserts the current position and then searches the chain
             * from the previous occurrence of the same hash, which is exactly
             * prev[strstart] once the chain is built. Starting at head[h]
             * instead would begin at a position AFTER the search point. */
            unsigned int cur_match = prev[strstart];
            unsigned int fast, slow;
            if (cur_match == NIL || cur_match >= strstart
                || strstart - cur_match > MAX_MATCH)
                continue;
            fast = longest_match(cur_match, strstart, MIN_MATCH - 1);
            slow = longest_match_reference(cur_match, strstart, MIN_MATCH - 1);
            checksum = mix(checksum, fast);
            reference = mix(reference, slow);
        }

    /* The fast path and the reference must agree; a mismatch is a compiler or
     * kernel defect, and the exit status carries it. */
    printf("gzip_longest_match checksum=%llu reference=%llu %s\n",
           checksum, reference, checksum == reference ? "OK" : "MISMATCH");
    return checksum == reference ? 0 : 1;
}
