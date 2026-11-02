/*
 * Restored session-8 regression (lost in the upstream merge): 3-operand
 * imul recurrence updates must not keep a stale source alive.
 *
 * fold_recurrence_update retargets the recurrence register of
 * `x = x * K` at -Os.  parse_inplace_op used to split the 3-op form
 * `imulq $7, %rsi, %rsi` at the LAST top-level comma, so the source group
 * `"$7, %rsi"` passed the `starts_with('$')` immediate check and the fold
 * retargeted the DEST while keeping the "separate" source — the deleted
 * defining copy made %rsi stale (`shiftchain_33`: got 87510588884992,
 * expected 17090488784267303466).
 *
 * Soundness contract (now enforced): src==dst folds to the 2-op form
 * (incorporates A_old); src!=dst is REFUSED (the 3-op form never reads its
 * dest — retargeting would clobber A_old before the commutative op reads
 * it).  27-case seed/multiplier matrix plus the exact stress shape; run at
 * -Os where the recurrence fold is active.
 */
#include <stdio.h>
#include <stdint.h>

/* The exact stress shape: one induction multiplier chain consumed by the
 * loop exit value, per-iteration mask forces the value through copies. */
__attribute__((noinline))
uint64_t shiftchain(uint64_t seed, uint64_t mul, int n) {
    uint64_t x = seed;
    for (int i = 0; i < n; i++) {
        x = x * mul;
        x ^= x >> 13;
        x += (uint64_t)i;
    }
    return x;
}

/* Recurrence whose update is a pure multiply of the loop-carried value. */
__attribute__((noinline))
uint64_t pure_mul(uint64_t seed, uint64_t mul, int n) {
    uint64_t x = seed;
    for (int i = 0; i < n; i++)
        x = x * mul;
    return x;
}

/* Two recurrences interleaved: retargeting one must not corrupt the other. */
__attribute__((noinline))
uint64_t dual_mul(uint64_t s1, uint64_t s2, uint64_t m1, uint64_t m2, int n) {
    uint64_t x = s1, y = s2;
    for (int i = 0; i < n; i++) {
        x = x * m1;
        y = y * m2 + (uint64_t)i;
    }
    return x ^ y;
}

static const uint64_t seeds[3] = {
    1ull, 0xDEADBEEFCAFEBABEull, 0x123456789ABCDEF0ull,
};
static const uint64_t muls[3][3] = {
    { 3ull, 7ull, 0xFFFFFFFFFFFFFFFFull }, /* include mul = -1 (negation) */
    { 5ull, 9ull, 0x8000000000000000ull },
    { 17ull, 25ull, 0x100000001ull },
};

int main(void) {
    /* 3 seeds x 3 multipliers x 3 kernels = 27 outcomes. */
    for (int s = 0; s < 3; s++) {
        for (int m = 0; m < 3; m++) {
            uint64_t mul = muls[s][m];
            printf("sc %d %d %llu\n", s, m,
                   (unsigned long long)shiftchain(seeds[s], mul, 33));
            printf("pm %d %d %llu\n", s, m,
                   (unsigned long long)pure_mul(seeds[s], mul, 21));
            printf("dm %d %d %llu\n", s, m,
                   (unsigned long long)dual_mul(seeds[s], seeds[(s + 1) % 3],
                                                mul, muls[(m + 2) % 3][m], 17));
        }
    }
    return 0;
}
