/* early_exit_compare_bounds.c - semantics fixture for unrolling an
   early-exit compare loop at *every* trip count and *every* exit position.

   Why this file exists (backlog.md CD-21, 2026-11-13).  The measured 1.205x
   deficit against Clang on zlib_ng_compare256 is entirely explained by Clang
   unrolling a constant-trip-count loop that has a data-dependent early
   return into straight-line blocks.  Any LCCC change that unrolls such a loop
   must be bit-exact for *all* exit positions, not just the ones a benchmark
   happens to hit: an unrolled version that gets the remainder, the exit
   value, or the final "no difference" case wrong still passes compare256 on
   its own fixture alphabet.

   The shape below is the compression-family idiom (compare256 / ZSTD_count /
   longest_match): walk the buffer in 8-byte words, return the length of the
   common prefix, and clamp at the buffer length.  It is checked against a
   byte-at-a-time reference over

     - every total length 0..64,
     - every first-difference position, including "no difference at all",
     - three difference classes (bit 0, bit 63 and 0xff), because a wide XOR
       reports the same first-difference byte for all three but a mistaken
       byte order or a ctz/clz mix-up does not, and
     - both same and different operand alignments, because an unrolled load
       may assume alignment the source does not guarantee.

   Nothing here is UB: both arrays are fully initialised and padded, every
   access is in bounds, and unsigned char comparison is exact.  Self-check:
   prints "ok (<n> cases)" and returns 0, or names the first failing case and
   returns 1.  A compiler that folds everything away still has to agree with
   the reference, so the test cannot be passed by deleting the work.
*/
#include <stdio.h>
#include <stdint.h>
#include <string.h>

#define MAXN 64

static unsigned char a[MAXN + 16];
static unsigned char b[MAXN + 16];

/* The shape under test: 8-byte word loop with a data-dependent early return. */
static unsigned compare_words(const unsigned char *x, const unsigned char *y,
                              unsigned n) {
    unsigned len = 0;
    while (len < n) {
        uint64_t xv, yv, diff;
        unsigned take = n - len;
        if (take > 8) take = 8;
        /* The arrays are padded by 16 bytes, so a full-word read is always in
           bounds even when `take` is smaller; bytes past `take` are then
           discarded below. */
        memcpy(&xv, x + len, sizeof xv);
        memcpy(&yv, y + len, sizeof yv);
        diff = xv ^ yv;
        if (diff) {
            unsigned pos = (unsigned)(__builtin_ctzll(diff) >> 3);
            return len + (pos < take ? pos : take);
        }
        len += take;
    }
    return n;
}

/* Byte-at-a-time ground truth. */
static unsigned reference(const unsigned char *x, const unsigned char *y,
                          unsigned n) {
    unsigned i;
    for (i = 0; i < n; i++)
        if (x[i] != y[i]) return i;
    return n;
}

static int check(unsigned n, int diff_at, unsigned char delta,
                 unsigned a_off, unsigned b_off) {
    unsigned i, got, want;
    for (i = 0; i < MAXN + 16; i++) {
        a[i] = (unsigned char)(i * 37u + 11u);
        b[i] = a[i];
    }
    if (diff_at >= 0)
        b[b_off + (unsigned)diff_at] =
            (unsigned char)(a[a_off + (unsigned)diff_at] ^ delta);
    got = compare_words(a + a_off, b + b_off, n);
    want = reference(a + a_off, b + b_off, n);
    if (got != want) {
        printf("FAIL n=%u diff_at=%d delta=0x%02x off=(%u,%u): got %u want %u\n",
               n, diff_at, (unsigned)delta, a_off, b_off, got, want);
        return 0;
    }
    return 1;
}

int main(void) {
    static const unsigned char deltas[] = {0x01u, 0x80u, 0xffu};
    unsigned n, d, align, failures = 0, cases = 0;

    /* Same alignment first (the common path). */
    for (align = 0; align < 9; align++) {
        for (n = 0; n <= MAXN; n++) {
            cases++;
            if (!check(n, -1, 0, align, align)) failures++;
            for (d = 0; d < n; d++) {
                unsigned k;
                for (k = 0; k < sizeof deltas; k++) {
                    cases++;
                    if (!check(n, (int)d, deltas[k], align, align)) failures++;
                }
            }
        }
    }
    /* Misaligned operands relative to each other: the unrolled-load hazard. */
    for (n = 1; n <= MAXN; n += 3) {
        unsigned a_off, b_off;
        for (a_off = 0; a_off < 4; a_off++) {
            for (b_off = 0; b_off < 4; b_off++) {
                if (a_off == b_off) continue;
                cases++;
                if (!check(n, -1, 0, a_off, b_off)) failures++;
                for (d = 0; d < n; d++) {
                    cases++;
                    if (!check(n, (int)d, 0x01u, a_off, b_off)) failures++;
                }
            }
        }
    }

    if (failures) {
        printf("early-exit compare: %u failure(s) in %u case(s)\n", failures, cases);
        return 1;
    }
    printf("ok (%u cases)\n", cases);
    return 0;
}
