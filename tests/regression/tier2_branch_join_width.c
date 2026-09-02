/* Tier-2 slot-sharing + small-slot width join regression.
 *
 * Pins two correctness properties that the 2026-09-02 preboot-ZSTD
 * corruption depended on ("ZSTD-compressed data is corrupt", errcode=20:
 * `movl %eax,80(%rsp)` on one CFG path, `movq 80(%rsp),%rax` at the join
 * reading a neighbour's stale high half):
 *
 *  1. A 32-bit value spilled into a small (4-byte) slot on one branch is
 *     re-read at full 64-bit width after a CFG join, repeatedly, with the
 *     neighbouring slot occupied by *different* live data on each arm — any
 *     width mismatch or unsound slot hand-off corrupts the accumulation.
 *  2. Values live in temporally disjoint regions of the function share stack
 *     slots (Tier-2 packing). The A/B differential in the harness (default
 *     vs CCC_NO_TIER2_GRAPH=1, and vs CCC_NO_SMALL_SLOTS=1) requires the
 *     packed layout to be value-identical to the conservative layout.
 *
 * The pattern is the *shape* of ZSTD_decodeLiteralsBlock: two exclusive
 * branches produce values that are consumed together after the merge, with
 * intervening calls that force caller-saved registers to spill.
 */
#include <stdio.h>
#include <stdint.h>

__attribute__((noinline)) static uint64_t clobber_all(uint64_t x) {
    /* Many caller-saved registers + flags touched; a real function call that
     * the compiler cannot see through. */
    return x * 6364136223846793005ULL + 1442695040888963407ULL;
}

__attribute__((noinline)) static int exercise(int seed, int n) {
    uint32_t lo = 0u, hi = 0u;      /* small 32-bit values, live across calls */
    uint64_t acc = 0;               /* 64-bit consumer of the joined value   */
    int out = 0;

    for (int i = 0; i < n; i++) {
        uint32_t a;
        if (seed & 1) {
            /* arm 1: fill a with a value whose high half is *not* zero */
            a = (uint32_t)(seed * 2654435761u) | 0x80000000u;
            lo = (uint32_t)clobber_all(a);      /* force a through the call   */
        } else {
            /* arm 2: different live data in the same frame region */
            uint32_t b = (uint32_t)(seed * 40503u) ^ 0xDEADBEEFu;
            hi = (uint32_t)clobber_all(b);
            a = hi;
        }
        /* join: read `a` at 64-bit width — the upper 32 bits must be a
         * correct zero-extension of the branch value, never neighbour data */
        uint64_t wide = (uint32_t)a;            /* explicit zero-extension   */
        uint64_t as_signed_consumer = (int32_t)a; /* sign-extension path      */
        acc += wide + (as_signed_consumer & 0xFFFFFFFFu);
        out += (int)(wide >> 31) + (int)(as_signed_consumer >> 32);
        seed = (int)clobber_all((uint64_t)(seed + i));
    }
    return (int)(acc & 0x7FFFFFFF) ^ out ^ (int)lo ^ (int)hi;
}

int main(void) {
    long h = 0;
    for (int s = 0; s < 64; s++) {
        h = h * 131 + exercise(s * 7919 + 13, 2000);
    }
    printf("tier2_branch_join h=%ld\n", h);
    return (h == 0) ? 1 : 0;
}
