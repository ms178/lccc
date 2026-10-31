/*
 * Pointer IV collapse: several pointer induction variables advancing by the
 * same constant, in a loop whose trip count is a separate literal-bounded
 * counter, may share one index register.
 *
 * The shape is zlib-ng's compare256_64 (arch/generic/compare256_p.h): a 64-bit
 * XOR compare that keeps `src0`, `src1` and `len` as three separate induction
 * variables.  Address generation then costs one `add` per pointer per
 * iteration; with a single index the reads become `(%rdi,%rdx)`/`(%rsi,%rdx)`
 * and the loop carries one counter.
 *
 * The rewrite is only legal when every pointer steps by the same constant and
 * its only other uses are the loads/stores that read through it, so this test
 * pins both directions:
 *
 *   1. the collapsible shapes produce byte-identical results to the reference
 *      byte loops (exhaustively, for every mismatch offset and every alignment
 *      of both operands);
 *   2. a loop whose bound is a runtime value, a loop whose pointers advance by
 *      different constants, and a loop that reads the pointer value itself
 *      (to compute a distance) must stay correct — the pass has to decline
 *      those, not miscompile them.
 *
 * Nothing here tests code generation shape; check_ptr_iv_collapse_codegen.sh
 * does that from the emitted assembly.  This file is the semantic half.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#ifndef PASSES
#define PASSES 64U
#endif

#define SPAN 264
#define MATCH_LENGTH 256U
#define SLACK 8U

static unsigned char left[SPAN];
static unsigned char right[SPAN];
static unsigned char third[SPAN];

/* The collapsible shape: two pointers + a literal-bounded counter. */
__attribute__((noinline)) static uint32_t compare256_64(const unsigned char *a,
                                                        const unsigned char *b) {
    uint32_t len = 0;
    do {
        uint64_t sv, mv, diff;
        memcpy(&sv, a, sizeof(sv));
        memcpy(&mv, b, sizeof(mv));
        diff = sv ^ mv;
        if (diff)
            return len + (uint32_t)((uint64_t)__builtin_ctzll(diff) / 8);
        a += 8;
        b += 8;
        len += 8;
    } while (len < MATCH_LENGTH);
    return MATCH_LENGTH;
}

/* Three arrays, one stride: the widened form (four pointer IVs at most). */
__attribute__((noinline)) static uint32_t sum3(const unsigned char *a,
                                               const unsigned char *b,
                                               const unsigned char *c) {
    uint32_t total = 0;
    for (uint32_t i = 0; i < MATCH_LENGTH; i += 8) {
        uint64_t x, y, z;
        memcpy(&x, a, sizeof(x));
        memcpy(&y, b, sizeof(y));
        memcpy(&z, c, sizeof(z));
        total += (uint32_t)(x ^ y) + (uint32_t)(z >> 32);
        a += 8;
        b += 8;
        c += 8;
    }
    return total;
}

/* Runtime bound: the trip count is not a literal, so no index can be
 * synthesised from it.  Must stay correct (and must be declined). */
__attribute__((noinline)) static uint32_t sum_n(const unsigned char *a,
                                                uint32_t n) {
    uint32_t total = 0;
    const unsigned char *p = a;
    for (uint32_t i = 0; i < n; i++) {
        total += *p;
        p++;
    }
    return total;
}

/* Different strides: a single index cannot serve both arrays. */
__attribute__((noinline)) static uint32_t mixed_stride(const unsigned char *a,
                                                       const uint32_t *b) {
    uint32_t total = 0;
    for (uint32_t i = 0; i < 16; i++) {
        total += a[i] + b[i];
    }
    return total;
}

/* The pointer value itself escapes into arithmetic: collapsing it would have
 * to keep the address, so the pass must leave this alone. */
__attribute__((noinline)) static unsigned long distance(const unsigned char *a,
                                                        const unsigned char *b) {
    const unsigned char *start = a;
    uint32_t len = 0;
    do {
        if (*a != *b)
            return (unsigned long)(a - start);
        a++;
        b++;
        len++;
    } while (len < 240);
    return (unsigned long)(a - start);
}

static uint32_t reference(const unsigned char *a, const unsigned char *b) {
    uint32_t n;
    for (n = 0; n < MATCH_LENGTH && a[n] == b[n]; n++)
        ;
    return n;
}

static int fill(void) {
    for (unsigned i = 0; i < SPAN; i++) {
        left[i] = (unsigned char)(i * 37U + (i >> 3) + 11U);
        right[i] = left[i];
        third[i] = (unsigned char)(i * 5U + 3U);
    }
    return 1;
}

int main(void) {
    uint64_t checksum = 1469598103934665603ULL;

    if (!fill())
        return 1;

    /* Every mismatch offset and every pairwise alignment of the two operands:
     * a collapsed index that dropped an offset would fail here. */
    for (unsigned la = 0; la < SLACK; la++) {
        for (unsigned ra = 0; ra < SLACK; ra++) {
            for (uint32_t pos = 0; pos <= MATCH_LENGTH; pos++) {
                memcpy(right + ra, left + la, MATCH_LENGTH);
                if (pos < MATCH_LENGTH)
                    right[ra + pos] ^= 0x80U;
                uint32_t want = reference(left + la, right + ra);
                uint32_t got = compare256_64(left + la, right + ra);
                if (want != got || want != pos) {
                    printf("compare256 mismatch: la=%u ra=%u pos=%u got=%u want=%u\n",
                           la, ra, pos, got, want);
                    return 2;
                }
                checksum = (checksum ^ (uint64_t)(got + pos + 1U)) * 1099511628211ULL;
            }
        }
    }

    /* Three-pointer form, against a scalar reference. */
    for (unsigned off = 0; off < SLACK; off++) {
        uint32_t want = 0;
        for (uint32_t i = 0; i < MATCH_LENGTH; i += 8) {
            uint64_t x, y, z;
            memcpy(&x, left + off + i, sizeof(x));
            memcpy(&y, right + off + i, sizeof(y));
            memcpy(&z, third + off + i, sizeof(z));
            want += (uint32_t)(x ^ y) + (uint32_t)(z >> 32);
        }
        uint32_t got = sum3(left + off, right + off, third + off);
        if (got != want) {
            printf("sum3 mismatch at off=%u: got=%u want=%u\n", off, got, want);
            return 3;
        }
        checksum = (checksum ^ (uint64_t)(got + off)) * 1099511628211ULL;
    }

    /* Runtime bound. */
    for (uint32_t n = 0; n <= 200; n += 7) {
        uint32_t want = 0;
        for (uint32_t i = 0; i < n; i++)
            want += left[i];
        if (sum_n(left, n) != want) {
            printf("sum_n mismatch at n=%u\n", n);
            return 4;
        }
    }

    /* Mixed strides. */
    {
        uint32_t words[16];
        for (unsigned i = 0; i < 16; i++)
            words[i] = i * 0x01010101U;
        uint32_t want = 0;
        for (unsigned i = 0; i < 16; i++)
            want += left[i] + words[i];
        if (mixed_stride(left, words) != want) {
            printf("mixed_stride mismatch\n");
            return 5;
        }
    }

    /* Escaping pointer value. */
    for (unsigned d = 0; d < 240; d++) {
        memcpy(right, left, SPAN);
        right[d] ^= 0x01U;
        unsigned long want = d < 240 ? d : 240;
        unsigned long got = distance(left, right);
        if (got != want) {
            printf("distance mismatch at d=%u: got=%lu want=%lu\n", d, got, want);
            return 6;
        }
    }

    /* Repeat the whole thing so the shapes are exercised from a loop, the way
     * a caller uses them (and so the pass sees non-first-iteration state). */
    for (unsigned pass = 0; pass < PASSES; pass++)
        checksum = (checksum ^ (uint64_t)compare256_64(left, left)) * 1099511628211ULL;

    printf("ptr_iv_collapse_scan checksum=%llu\n", (unsigned long long)checksum);
    return 0;
}
