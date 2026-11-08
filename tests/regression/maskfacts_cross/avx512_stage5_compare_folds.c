/* Round-8 stage-5 three-way differential: LCCC / GCC 16.2 / Clang 23.1.5.
 *
 * Exercises the newly unlocked 512-bit SCALAR-MASK compare folds.  Before
 * stage 5 `mask_compare_shape` listed only 128/256-bit compares and the
 * fold refused 64 lanes outright (`lane_count > 63`), so every one of the
 * functions below kept a `vpcmp* + kmovq` pair even when the answer is a
 * compile-time constant.  `MAX_IMAGE = 64` now proves a whole ZMM object
 * and `ScalarMask::Mixed(u64)` covers exactly 64 lanes.
 *
 * Two independent proofs are printed:
 *   (1) self-identity: `cmpeq(x,x) == u64::MAX`, `cmpne(x,x) == 0`
 *       for a RUNTIME vector (no constant image needed at all);
 *   (2) constant objects: two 64-byte stack arrays with byte stores, whose
 *       per-lane equality is recomputed by a scalar C oracle.  LCCC's image
 *       proof is width-exact; GCC/Clang compute the same mask at runtime.
 *
 * Build:
 *   -O2 -mavx512f -mavx512bw -mavx512vl -mavx512dq -I include
 *
 * This directory is NOT globbed by tests/regression/run_regression.py (which
 * is intentionally non-recursive and SSE2-safe for hosted runners); it is a
 * local differential fixture, exactly like avx512_stage2_folds.c.
 */
#include <stdio.h>
#include <string.h>
#include <immintrin.h>

static unsigned char src[64];

/* A RUNTIME vector: neither compiler may assume anything about its bytes,
 * yet x == x is true in every lane and x != x is false in every lane. */
__attribute__((noinline)) static unsigned long long self_eq(void) {
    __m512i v = _mm512_loadu_si512((const void *)src);
    return (unsigned long long)_mm512_cmpeq_epi8_mask(v, v);
}
__attribute__((noinline)) static unsigned long long self_ne(void) {
    __m512i v = _mm512_loadu_si512((const void *)src);
    return (unsigned long long)_mm512_cmp_epi8_mask(v, v, 4);
}
__attribute__((noinline)) static unsigned long long self_eq16(void) {
    __m512i v = _mm512_loadu_si512((const void *)src);
    return (unsigned long long)_mm512_cmpeq_epi16_mask(v, v);
}
__attribute__((noinline)) static unsigned long long self_eq32(void) {
    __m512i v = _mm512_loadu_si512((const void *)src);
    return (unsigned long long)_mm512_cmpeq_epi32_mask(v, v);
}
__attribute__((noinline)) static unsigned long long self_eq64(void) {
    __m512i v = _mm512_loadu_si512((const void *)src);
    return (unsigned long long)_mm512_cmpeq_epi64_mask(v, v);
}

/* Constant 64-byte images: lane 1 and lane 63 differ, every other lane is
 * equal.  The mask is therefore 0xFFFF...FFFF with bits 1 and 63 cleared. */
__attribute__((noinline)) static unsigned long long const_eq8(void) {
    unsigned char a[64], b[64];
    for (int i = 0; i < 64; i++) { a[i] = (unsigned char)(i * 5 + 3); b[i] = a[i]; }
    b[1] ^= 0x40;
    b[63] ^= 0x80;
    __m512i va = _mm512_loadu_si512((const void *)a);
    __m512i vb = _mm512_loadu_si512((const void *)b);
    return (unsigned long long)_mm512_cmpeq_epi8_mask(va, vb);
}
__attribute__((noinline)) static unsigned long long const_ne8(void) {
    unsigned char a[64], b[64];
    for (int i = 0; i < 64; i++) { a[i] = (unsigned char)(i * 7 + 1); b[i] = a[i]; }
    b[1] ^= 0x40;
    b[63] ^= 0x80;
    __m512i va = _mm512_loadu_si512((const void *)a);
    __m512i vb = _mm512_loadu_si512((const void *)b);
    return (unsigned long long)_mm512_cmp_epi8_mask(va, vb, 4);
}

/* Independent scalar oracle over the very same byte arrays. */
static unsigned long long oracle8(int lane_bytes, int ne) {
    unsigned char a[64], b[64];
    for (int i = 0; i < 64; i++) {
        a[i] = (unsigned char)(i * (lane_bytes == 1 && ne ? 7 : 5)
                               + (ne ? 1 : 3));
        b[i] = a[i];
    }
    b[1] ^= 0x40;
    b[63] ^= 0x80;
    unsigned long long m = 0;
    int lanes = 64 / lane_bytes;
    for (int i = 0; i < lanes; i++) {
        int equal = 1;
        for (int j = 0; j < lane_bytes; j++)
            if (a[i * lane_bytes + j] != b[i * lane_bytes + j]) equal = 0;
        if ((equal != 0) ^ (ne != 0)) m |= 1ULL << i;
    }
    return m;
}

int main(void) {
    for (int i = 0; i < 64; i++) src[i] = (unsigned char)(i * 11 + 7);

    unsigned long long eq8 = self_eq();
    unsigned long long ne8 = self_ne();
    unsigned long long eq16 = self_eq16();
    unsigned long long eq32 = self_eq32();
    unsigned long long eq64 = self_eq64();
    unsigned long long ceq = const_eq8();
    unsigned long long cne = const_ne8();

    /* Self-identity: exact masks for every lane width.  These are
     * hard constants, so even a compiler that refuses the fold must
     * produce the same stdout. */
    if (eq8 != ~0ULL || ne8 != 0ULL ||
        eq16 != 0xFFFFFFFFULL || eq32 != 0xFFFFULL || eq64 != 0xFFULL) {
        printf("SELF-IDENTITY MISMATCH\n");
        return 1;
    }
    if (ceq != oracle8(1, 0) || cne != oracle8(1, 1)) {
        printf("CONSTANT MISMATCH %016llx %016llx\n", ceq, cne);
        return 1;
    }
    /* Bit 63 is the boundary shift; prove the oracle itself reaches it. */
    if ((oracle8(1, 0) & (1ULL << 63)) != 0 ||
        (oracle8(1, 0) & (1ULL << 1)) != 0) {
        printf("ORACLE MISMATCH\n");
        return 1;
    }

    printf("%016llx %016llx %016llx %016llx %016llx %016llx %016llx\n",
           eq8, ne8, eq16, eq32, eq64, ceq, cne);
    return 0;
}
