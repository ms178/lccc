/*
 * 64-bit predicates over spilled 32-bit values (kernel lib/zstd
 * ZSTD_decodeLiteralsBlock shape: `if (!lhlCode)` with lhlCode resident in
 * a 4-byte small slot).  The backend must never substitute a 4-byte stack
 * slot into an 8-byte memory-operand compare: `cmpq $0, slot(%rsp)` reads
 * 8 bytes and picks up the adjacent slot's bytes as the high half.
 *
 * Twelve live 32-bit locals per iteration force small-slot spilling; the
 * 64-bit ! / == / relational predicates must observe exactly the stored 4
 * bytes.  CCC_NO_SMALL_SLOTS=1 (suite A/B) re-runs this with 8-byte slots.
 */
#include <stdio.h>

#define NOINLINE __attribute__((noinline))

static unsigned seed = 0x12345678u;
NOINLINE unsigned next(void) {
    seed = seed * 1664525u + 1013904223u;
    return seed;
}

NOINLINE unsigned decide(unsigned a, unsigned b, unsigned c, unsigned d) {
    unsigned v0 = a, v1 = b, v2 = c, v3 = d;
    unsigned v4 = a ^ b, v5 = b + c, v6 = c | d, v7 = d & a;
    unsigned v8 = a - b, v9 = b * 3u, v10 = c >> 1, v11 = d << 1;
    unsigned acc = 0;
    /* every predicate is a 64-bit-truthiness question about a 32-bit value */
    if (!v0) acc += 1;
    if (!v1) acc += 2;
    if (!v4) acc += 4;
    if (!v8) acc += 8;
    if (!(v5 ^ 0xdeadbeefu)) acc += 16;
    if ((unsigned long long)v6 == (unsigned long long)v7) acc += 32;
    if ((unsigned long long)v9 > (unsigned long long)0xffff0000u) acc += 64;
    if (!(v10 | v11)) acc += 128;
    /* keep all twelve live across the predicate cluster */
    acc += v0 + v1 + v2 + v3 + v4 + v5 + v6 + v7 + v8 + v9 + v10 + v11;
    acc &= 0xffffu;
    return acc;
}

int main(void) {
    unsigned h = 0;
    for (int i = 0; i < 2000; i++) {
        /* Sequenced explicitly: the order of next() calls within one
         * argument list is unspecified in C and may legitimately differ
         * between compilers. */
        unsigned a = next(), b = next(), c = next(), d = next();
        h = h * 31 + decide(a, b, c, d);
        /* biased values to hit the zero branches */
        if ((i & 3) == 0) h = h * 31 + decide(0, 0, 0, 0);
        if ((i & 7) == 1) h = h * 31 + decide(0xdeadbeefu, 0, 0xffffffffu, 0);
    }
    printf("%08x\n", h);
    return 0;
}
