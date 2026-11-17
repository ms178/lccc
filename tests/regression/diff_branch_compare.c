/* Difference-branch canonicalization: `if (a ^ b)` / `if (a - b)` /
 * `(a ^ b) != 0` become `a != b` (bit_idioms diff_branch), and a dominating
 * `a != b` edge promotes `ctz(a ^ b)` to its nonzero contract (cvp
 * difference_proves_nonzero).
 *
 * Two things are locked:
 *  1. VALUE: every rewritten kernel computes the same answer as the portable
 *     spelling — match lengths at every byte offset, mismatches confined to
 *     the high word half (bits 32..63, where a narrowed compare would lie),
 *     and full-length matches that leave the loop by the trip count.
 *  2. PROMOTION EDGES: `ctz(a ^ b)` is only ever evaluated here where C
 *     defines it — on an edge proving a != b (the promotion's home), or
 *     behind a stop-bit OR guard where the pair may be equal. (`ctzll(0)`
 *     itself is undefined at the C level, so the equal-edge polarity is
 *     locked by cvp's unit tests, not by this program.) The FALSE edge of
 *     a non-strict ordering (`a <= b` false, hence a > b) is the one that
 *     proves disequality.
 *
 * Output is a fixed set of decimal lines, diffed against a reference
 * compiler build by the regression suite.
 */
#include <stdio.h>
#include <stdint.h>
#include <string.h>

/* zlib-ng compare256: branch directly on the XOR difference. */
__attribute__((noinline)) static uint32_t
compare256_xor(const unsigned char *s0, const unsigned char *s1)
{
    uint32_t len = 0;
    do {
        uint64_t sv, mv, diff;
        memcpy(&sv, s0, sizeof(sv));
        memcpy(&mv, s1, sizeof(mv));
        diff = sv ^ mv;
        if (diff)
            return len + (uint32_t)((uint64_t)__builtin_ctzll(diff) >> 3);
        s0 += 8, s1 += 8, len += 8;
    } while (len < 256u);
    return 256u;
}

/* The post-rewrite spelling: branch on disequality, difference kept for the
 * mismatch arm only. Must agree with compare256_xor on every input. */
__attribute__((noinline)) static uint32_t
compare256_ne(const unsigned char *s0, const unsigned char *s1)
{
    uint32_t len = 0;
    do {
        uint64_t sv, mv, diff;
        memcpy(&sv, s0, sizeof(sv));
        memcpy(&mv, s1, sizeof(mv));
        diff = sv ^ mv;
        if (sv != mv)
            return len + (uint32_t)((uint64_t)__builtin_ctzll(diff) >> 3);
        s0 += 8, s1 += 8, len += 8;
    } while (len < 256u);
    return 256u;
}

/* Branch on a wrapping subtraction: zero exactly on equal inputs. */
__attribute__((noinline)) static uint32_t
sub_branch(uint32_t a, uint32_t b)
{
    uint32_t d = a - b;
    if (d)
        return (uint32_t)__builtin_ctz(d);
    return 99u;
}

/* Explicit `(a ^ b) != 0` materialized as a value, not a branch. */
__attribute__((noinline)) static uint32_t
xor_ne_value(uint64_t a, uint64_t b)
{
    return ((a ^ b) != 0) + 2u * (0 == (a ^ b));
}

/* The promotion's home edge: a != b proves the difference nonzero, so the
 * bare ctz is defined and must compute the true bit index. */
__attribute__((noinline)) static uint32_t
ctz_on_unequal_edge(uint64_t a, uint64_t b)
{
    if (a != b)
        return (uint32_t)__builtin_ctzll(a ^ b);
    return 77u;
}

/* PROMOTION EDGES under an ordering: `a <= b` FALSE is a > b, which proves
 * a != b — the else-arm's bare ctz may use the nonzero contract and must
 * still compute the true bit index. The then-arm includes a == b, so its
 * ctz reads through a stop-bit guard that keeps it defined (and that no
 * promotion may discard without the disequality proof). */
__attribute__((noinline)) static uint64_t
ctz_under_ordering(uint64_t a, uint64_t b)
{
    if (a <= b)
        return 100u + (uint32_t)__builtin_ctzll((a ^ b) | 0x10000u);
    return (uint32_t)__builtin_ctzll(a ^ b);
}

/* A narrowed compare must NOT stand in for the wide difference: the bytes
 * here agree in the low 32 bits and differ only above them. */
__attribute__((noinline)) static uint32_t
high_half_only(uint64_t a, uint64_t b)
{
    uint64_t d = a ^ b;
    if (d)
        return (uint32_t)((uint64_t)__builtin_ctzll(d) >> 3);
    return 255u;
}

int main(void)
{
    static unsigned char s0[256 + 8], s1[256 + 8];
    uint32_t state = 0x85467329u, agree = 1;

    for (unsigned i = 0; i < sizeof(s0); i++) {
        state = state * 1664525u + 1013904223u;
        s0[i] = (unsigned char)(state >> 24);
        s1[i] = s0[i];
    }

    /* Mismatch planted at every byte position, including none at all. */
    uint64_t sum = 0;
    for (unsigned pos = 0; pos <= 256; pos++) {
        unsigned char saved = 0;
        if (pos < 256) {
            saved = s1[pos];
            s1[pos] = (unsigned char)(saved ^ 0x40u);
        }
        uint32_t a = compare256_xor(s0, s1);
        uint32_t b = compare256_ne(s0, s1);
        agree &= (a == b);
        sum += a;
        if (pos < 256)
            s1[pos] = saved;
    }
    printf("compare256 sum=%llu agree=%u\n", (unsigned long long)sum, agree);

    printf("sub %u %u %u\n", sub_branch(12u, 12u), sub_branch(12u, 4u),
           sub_branch(0u, 1u));
    printf("ne_value %u %u\n", xor_ne_value(5, 5), xor_ne_value(5, 7));
    printf("unequal_edge %u %u\n", ctz_on_unequal_edge(9, 9),
           ctz_on_unequal_edge(9, 8));
    printf("ordering %llu %llu %llu\n",
           (unsigned long long)ctz_under_ordering(3, 3),
           (unsigned long long)ctz_under_ordering(3, 11),
           (unsigned long long)ctz_under_ordering(11, 3));
    printf("high_half %u %u\n", high_half_only(0x0123456789abcdefull,
                                               0x1123456789abcdefull),
           high_half_only(42, 42));
    return 0;
}
