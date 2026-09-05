/*
 * Restored session-8 regression (lost in the upstream merge): peephole
 * death proofs must not use fixed lookahead windows.
 *
 * fold_cascaded_shifts fused `shl $17,t1; mov t1,t2; shl $8,t2` into
 * `shl $25,t2` after declaring t1 dead from an 8-LINE forward window — an
 * inversion of soundness: the genuine use of t1 sits 13 lines down the same
 * block, beyond any fixed window (and the window scan also accepted
 * read-modify-writes as redefinitions).  The fix routes register death
 * through the exact per-line liveness oracle (compute_gpr_live_out), which
 * sees reads however far away and through redefinitions.
 *
 * This file pins the far-use shape: the cascaded shift chain is followed by
 * a run of independent filler operations, and the INTERMEDIATE shift value
 * (t1) is consumed only after the window would have expired.  Every kernel
 * is UB-free (unsigned 64-bit) and self-checking; run at -O1 where the
 * peephole pipeline is active.
 */
#include <stdio.h>
#include <stdint.h>

__attribute__((noinline))
uint64_t far_use(uint64_t a, uint64_t b) {
    uint64_t t1 = a << 17;
    uint64_t t2 = t1 << 8; /* cascaded fold target (== a << 25) */
    /* 6 independent filler ops: their only live value is s. */
    uint64_t s = b * 3u + 7u;
    s ^= s >> 3;
    s += (uint64_t)0x9E3779B97F4A7C15ull;
    s *= 0x2545F4914F6CDD1Dull;
    s ^= s << 5;
    s -= b | 1u;
    /* Far consumer of t1: 13+ lines below its definition. */
    return (t2 ^ s) + (t1 & b) + (t2 >> 2);
}

/* Multi-payload: two independent cascaded chains interleaved, both consumed
 * far below; a window-based scan may kill the wrong one. */
__attribute__((noinline))
uint64_t far_use_two(uint64_t a, uint64_t b, uint64_t c) {
    uint64_t p1 = a << 5;
    uint64_t q1 = p1 << 11; /* == a << 16 */
    uint64_t filler = c ^ (b >> 7);
    uint64_t p2 = b << 3;
    uint64_t q2 = p2 << 19; /* == b << 22 */
    uint64_t mix = filler * 5u + (uint64_t)(int)(c & 0xFFu);
    mix ^= mix >> 2;
    /* Far uses: p1 and p2 outlive their windows; q1/q2 fold. */
    return (q1 + mix) ^ (p1 * 3u) + (q2 - filler) + (p2 & c) + q1;
}

/* Read-modify-write must count as a READ (window scans treated
 * `Other{dest==tmp}` as a redefinition): the += below keeps t alive. */
__attribute__((noinline))
uint64_t rmw_keeps_alive(uint64_t a, uint64_t b) {
    uint64_t t = a << 9;
    uint64_t t2 = t << 4; /* fold candidate */
    uint64_t s = b ^ 0xFF00FF00ull;
    s = s * 7u + (s >> 9);
    s ^= s << 11;
    t += s; /* RMW read of t */
    return t2 ^ t;
}

static uint64_t seed = 0x123456789ABCDEF0ull;
static uint64_t next(void) {
    seed ^= seed << 13;
    seed ^= seed >> 7;
    seed ^= seed << 17;
    return seed;
}

int main(void) {
    for (int i = 0; i < 200; i++) {
        uint64_t a = next(), b = next(), c = next();
        printf("fu1 %llu\n", (unsigned long long)far_use(a, b));
        printf("fu2 %llu\n", (unsigned long long)far_use_two(a, b, c));
        printf("rmw %llu\n", (unsigned long long)rmw_keeps_alive(a, b));
    }
    return 0;
}
