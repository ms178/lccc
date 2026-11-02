/* Tier-2 slot-sharing stress: many multi-block SSA values whose live ranges
 * are temporally disjoint share stack slots under the packed layout.
 *
 * Sixteen values live through the FIRST call region (v0..v15), then die;
 * sixteen more live through the SECOND call region (w0..w15). The packed
 * colorer (Tier-2) hands the w's the slots the v's vacated; a colouring bug
 * that lets a still-live value share its slot corrupts the checksum. Each
 * value is deliberately wide (uint64) and fed through an opaque call so the
 * compiler must keep it in a stack home across the call, and each region is
 * long enough to force many simultaneous spills.
 *
 * The A/B differential in the harness compares this against
 * CCC_NO_TIER2_GRAPH=1 (conservative one-slot-per-value) and
 * CCC_NO_SMALL_SLOTS=1; all three layouts must agree exactly.
 */
#include <stdio.h>
#include <stdint.h>

__attribute__((noinline)) static uint64_t opaque(uint64_t x, uint64_t y) {
    /* opaque call: clobbers all caller-saved registers */
    uint64_t z = x ^ (y * 0x9E3779B97F4A7C15ULL);
    for (int i = 0; i < 3; i++) z = (z >> 33) ^ (z << 31);
    return z;
}

__attribute__((noinline)) static uint64_t stress(uint64_t seed, int n) {
    uint64_t v0 = seed + 0,  v1 = seed + 1,  v2 = seed + 2,  v3 = seed + 3;
    uint64_t v4 = seed + 4,  v5 = seed + 5,  v6 = seed + 6,  v7 = seed + 7;
    uint64_t v8 = seed + 8,  v9 = seed + 9,  v10 = seed + 10, v11 = seed + 11;
    uint64_t v12 = seed + 12, v13 = seed + 13, v14 = seed + 14, v15 = seed + 15;

    /* region 1: all sixteen v's live simultaneously through a call body */
    uint64_t a = seed;
    for (int i = 0; i < n; i++) {
        a = opaque(a, v0) ^ opaque(a, v1) ^ opaque(a, v2) ^ opaque(a, v3)
          ^ opaque(a, v4) ^ opaque(a, v5) ^ opaque(a, v6) ^ opaque(a, v7);
        v0 = opaque(v0, a); v1 = opaque(v1, a); v2 = opaque(v2, a); v3 = opaque(v3, a);
        v4 = opaque(v4, a); v5 = opaque(v5, a); v6 = opaque(v6, a); v7 = opaque(v7, a);
        v8 ^= a; v9 ^= a; v10 ^= a; v11 ^= a; v12 ^= a; v13 ^= a; v14 ^= a; v15 ^= a;
    }
    /* fold the region-1 results while the v's are still live */
    uint64_t fold1 = v0 ^ v1 ^ v2 ^ v3 ^ v4 ^ v5 ^ v6 ^ v7
                   ^ v8 ^ v9 ^ v10 ^ v11 ^ v12 ^ v13 ^ v14 ^ v15 ^ a;

    /* region 2: fresh w's that can reuse the v's vacated slots */
    uint64_t w0 = fold1 + 0,  w1 = fold1 + 1,  w2 = fold1 + 2,  w3 = fold1 + 3;
    uint64_t w4 = fold1 + 4,  w5 = fold1 + 5,  w6 = fold1 + 6,  w7 = fold1 + 7;
    uint64_t w8 = fold1 + 8,  w9 = fold1 + 9,  w10 = fold1 + 10, w11 = fold1 + 11;
    uint64_t w12 = fold1 + 12, w13 = fold1 + 13, w14 = fold1 + 14, w15 = fold1 + 15;

    uint64_t b = a ^ fold1;
    for (int i = 0; i < n; i++) {
        b = opaque(b, w0) ^ opaque(b, w1) ^ opaque(b, w2) ^ opaque(b, w3)
          ^ opaque(b, w4) ^ opaque(b, w5) ^ opaque(b, w6) ^ opaque(b, w7);
        w0 = opaque(w0, b); w1 = opaque(w1, b); w2 = opaque(w2, b); w3 = opaque(w3, b);
        w4 = opaque(w4, b); w5 = opaque(w5, b); w6 = opaque(w6, b); w7 = opaque(w7, b);
        w8 ^= b; w9 ^= b; w10 ^= b; w11 ^= b; w12 ^= b; w13 ^= b; w14 ^= b; w15 ^= b;
    }
    uint64_t fold2 = w0 ^ w1 ^ w2 ^ w3 ^ w4 ^ w5 ^ w6 ^ w7
                   ^ w8 ^ w9 ^ w10 ^ w11 ^ w12 ^ w13 ^ w14 ^ w15 ^ b;
    return fold2;
}

int main(void) {
    uint64_t h = 0;
    for (int s = 0; s < 8; s++) {
        h ^= stress(0x123456789ABCDEF0ULL ^ (uint64_t)s * 0x9E37ULL, 300);
    }
    printf("tier2_sharing h=%016llx\n", (unsigned long long)h);
    return 0;
}
