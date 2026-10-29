/* Unsigned induction-variable and unsigned-lane reduction vectorization.
 *
 * Before the unsigned reduction work the reduction analyzer required a
 * width-matched SIGNED strict exit (I32/I64 Slt/Sgt) and signed element
 * types, so every systems-code idiom written with `size_t`/`unsigned`
 * counters or uint32_t/uint64_t accumulators stayed scalar — the single
 * most frequent reduction reject across the benchmark corpus. The signed
 * and unsigned packed integer add/multiply-low operations are the same
 * two's-complement bit operations, so the same vector lanes apply; the
 * unsigned counter only changes the bound division (UDiv/LShr) and the
 * scalar remainder's exit predicate (Ult instead of Slt).
 *
 * Soundness fences pinned by this file:
 *  - high-bit-set data on every lane would expose an accidental
 *    sign-extension, so the data below deliberately lives above 2^31;
 *  - widening an unsigned U32 element into a U64 accumulator has no
 *    zero-extending vector composite and must stay scalar;
 *  - an unsigned guarded sum (Ugt) has no masked composite and stays scalar;
 *  - an inclusive Ule bound is outside the strict-exit grammar and stays
 *    scalar. All three refusals must still print the exact reference answer.
 *
 * Dynamic runtime bounds sweep every remainder shape around the SSE2
 * width (2) and the AVX2 width (4); check_unsigned_reduction_codegen.sh
 * pins the vector/scalar split on both -march levels.
 */
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>

static uint32_t a32[1024];
static uint32_t b32[1024];
static uint64_t a64[1024];
static uint64_t b64[1024];
static int32_t s32[1024];

/* U64 IV, U32 lanes/accumulator: the `size_t i` loop idiom. */
uint32_t sum_u32_u64iv(const uint32_t *a, size_t n) {
    uint32_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

/* U32 IV, U32 lanes. */
uint32_t sum_u32_u32iv(const uint32_t *a, unsigned n) {
    uint32_t s = 0;
    for (unsigned i = 0; i < n; i++) s += a[i];
    return s;
}

/* U64 IV, U64 lanes (64-bit accumulation). */
uint64_t sum_u64(const uint64_t *a, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

/* Same-width unsigned dot products. */
uint32_t dot_u32(const uint32_t *a, const uint32_t *b, size_t n) {
    uint32_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}

uint64_t dot_u64(const uint64_t *a, const uint64_t *b, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i] * b[i];
    return s;
}

/* Signed control on the same machinery; must remain exactly as before. */
int32_t sum_i32(const int32_t *a, int n) {
    int32_t s = 0;
    for (int i = 0; i < n; i++) s += a[i];
    return s;
}

/* Reversed strict polarity (`n > i`); stays scalar today, must be correct. */
uint32_t sum_u32_gt(const uint32_t *a, size_t n) {
    uint32_t s = 0;
    size_t i = 0;
    while (n > i) {
        s += a[i];
        i++;
    }
    return s;
}

/* Unsigned widening: no zero-extending vector composite, stays scalar. */
uint64_t widen_u32_to_u64(const uint32_t *a, size_t n) {
    uint64_t s = 0;
    for (size_t i = 0; i < n; i++) s += (uint64_t)a[i];
    return s;
}

/* Unsigned guard: no unsigned masked composite, stays scalar. */
uint32_t guarded_u32(const uint32_t *a, size_t n, uint32_t k) {
    uint32_t s = 0;
    for (size_t i = 0; i < n; i++)
        if (a[i] > k) s += a[i];
    return s;
}

/* Inclusive unsigned bound: outside the strict-exit grammar, stays scalar. */
uint32_t sum_u32_le(const uint32_t *a, size_t n) {
    uint32_t s = 0;
    for (size_t i = 0; i <= n; i++) s += a[i];
    return s;
}

static uint32_t ref_sum32(const uint32_t *a, size_t n) {
    uint32_t s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}

int main(void) {
    /* Every datum keeps the high bit set so a sign-extending lane would
     * change the answer immediately. */
    for (size_t i = 0; i < 1024; i++) {
        a32[i] = 0x80000000u + (uint32_t)(i * 2654435761u);
        b32[i] = 0xFFFF0000u ^ (uint32_t)(i * 40503u);
        a64[i] = 0x8000000000000000ull | ((uint64_t)a32[i] << 7) | i;
        b64[i] = 0xF000000000000000ull | (uint64_t)b32[i];
        s32[i] = (int32_t)a32[i];
    }

    static const size_t ns[] = {
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 16, 17, 31, 32, 33,
        63, 64, 65, 127, 128, 129, 255, 256, 257, 511, 512, 1000, 1023
    };
    for (size_t k = 0; k < sizeof(ns) / sizeof(ns[0]); k++) {
        size_t n = ns[k];

        if (sum_u32_u64iv(a32, n) != ref_sum32(a32, n)) {
            printf("FAIL sum_u32_u64iv n=%zu\n", n);
            return 1;
        }
        if (sum_u32_u32iv(a32, (unsigned)n) != ref_sum32(a32, n)) {
            printf("FAIL sum_u32_u32iv n=%zu\n", n);
            return 2;
        }
        if (sum_u32_gt(a32, n) != ref_sum32(a32, n)) {
            printf("FAIL sum_u32_gt n=%zu\n", n);
            return 3;
        }

        uint64_t r64 = 0;
        for (size_t i = 0; i < n; i++) r64 += a64[i];
        if (sum_u64(a64, n) != r64) {
            printf("FAIL sum_u64 n=%zu\n", n);
            return 4;
        }

        uint32_t rd32 = 0;
        for (size_t i = 0; i < n; i++) rd32 += a32[i] * b32[i];
        if (dot_u32(a32, b32, n) != rd32) {
            printf("FAIL dot_u32 n=%zu\n", n);
            return 5;
        }
        uint64_t rd64 = 0;
        for (size_t i = 0; i < n; i++) rd64 += a64[i] * b64[i];
        if (dot_u64(a64, b64, n) != rd64) {
            printf("FAIL dot_u64 n=%zu\n", n);
            return 6;
        }

        int32_t rs = 0;
        for (int i = 0; i < (int)n; i++) rs += s32[i];
        if (sum_i32(s32, (int)n) != rs) {
            printf("FAIL sum_i32 n=%zu\n", n);
            return 7;
        }

        uint64_t rw = 0;
        for (size_t i = 0; i < n; i++) rw += (uint64_t)a32[i];
        if (widen_u32_to_u64(a32, n) != rw) {
            printf("FAIL widen n=%zu\n", n);
            return 8;
        }

        uint32_t rg = 0;
        for (size_t i = 0; i < n; i++)
            if (a32[i] > 0x90000000u) rg += a32[i];
        if (guarded_u32(a32, n, 0x90000000u) != rg) {
            printf("FAIL guarded n=%zu\n", n);
            return 9;
        }

        size_t le_n = n < 1023 ? n : 1023;
        uint32_t rle = 0;
        for (size_t i = 0; i <= le_n; i++) rle += a32[i];
        if (sum_u32_le(a32, le_n) != rle) {
            printf("FAIL le n=%zu\n", n);
            return 10;
        }
    }

    printf("OK unsigned reduction battery\n");
    return 0;
}
