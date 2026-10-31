/*
 * Portable zero guards around the scalar bit-count intrinsics.
 *
 * `__builtin_clz(x | 1)` and `__builtin_ctz(x | 0x80000000)` exist precisely
 * to make the undefined input defined: the OR forces a bit on, so the operand
 * of the count can never be zero — for every value of x, including the x == 0
 * case the guard was written for.  Without that fact the baseline x86 lowering
 * has to emit the defined-zero diamond (test / jnz / mov $width / jmp / bsr /
 * xor) even though its zero arm is unreachable.
 *
 * The outputs below are total functions of x, so this test is a pure
 * differential: every value must agree with the C reference, and the reference
 * is written out longhand so it cannot be confused with the intrinsic's own
 * contract.
 *
 * The interesting part is not that the values are right (they must be, with or
 * without the fold) but that the *folded* shapes still are: the guarded byte
 * bit-length wrapper, the capped forms, the mixed clz/ctz min, and the
 * unguarded `clz(0)`/`ctz(0)` cases that must keep the defined-width answer.
 * check_clz_zero_guard_codegen.sh pins the instruction selection itself.
 */
#include <stdint.h>
#include <stdio.h>

/* Longhand references: defined for every x, no undefined behaviour, no
 * intrinsic involved. */
static unsigned ref_clz32(uint32_t x) {
    unsigned n = 0;
    if (x == 0)
        return 32;
    uint32_t bit = 0x80000000U;
    while (!(x & bit)) {
        n++;
        bit >>= 1;
    }
    return n;
}

static unsigned ref_ctz32(uint32_t x) {
    unsigned n = 0;
    if (x == 0)
        return 32;
    while (!(x & 1U)) {
        n++;
        x >>= 1;
    }
    return n;
}

/* Guarded forms: the operand is provably nonzero. */
__attribute__((noinline)) static unsigned clz_guarded(uint32_t x) {
    return (unsigned)__builtin_clz(x | 1U);
}

__attribute__((noinline)) static unsigned ctz_guarded(uint32_t x) {
    return (unsigned)__builtin_ctz(x | 0x80000000U);
}

/* The reviewed byte bit-length spelling: 1 + (clz(x|1) ^ 31), truncated to a
 * byte.  Equals 32 - clz(x|1) == bsr(x|1) + 1. */
__attribute__((noinline)) static unsigned byte_bit_length(uint32_t x) {
    return (uint8_t)((uint32_t)(__builtin_clz(x | 1U) ^ 31U) + 1U);
}

/* Capped forms: with the cap folded into the guard bit the whole thing is one
 * scan, so the cap must not change the answer. */
__attribute__((noinline)) static unsigned cap_trailing(uint32_t x) {
    unsigned a = (unsigned)__builtin_ctz(x | 0x80000000U);
    return a < 6 ? a : 6;
}

__attribute__((noinline)) static unsigned cap_leading(uint32_t x) {
    unsigned a = (unsigned)__builtin_clz(x | 1U);
    return a < 6 ? a : 6;
}

/* Mixed widths in one expression: ctz of one operand beside clz of another. */
__attribute__((noinline)) static unsigned mixed_min(uint32_t x, uint32_t y) {
    unsigned a = (unsigned)__builtin_ctz(x | 0x80000000U);
    unsigned b = (unsigned)__builtin_clz(y | 1U);
    return a < b ? a : b;
}

/* Unguarded: undefined at zero in C, so this file never calls them with zero.
 * They exist here to keep the defined-zero path exercised. */
__attribute__((noinline)) static unsigned clz_plain(uint32_t x) {
    return (unsigned)__builtin_clz(x);
}

__attribute__((noinline)) static unsigned ctz_plain(uint32_t x) {
    return (unsigned)__builtin_ctz(x);
}

/* 64-bit guard, including the narrow-result cast the frontend inserts. */
__attribute__((noinline)) static unsigned clz64_guarded(uint64_t x) {
    return (unsigned)__builtin_clzll(x | 1ULL);
}

int main(void) {
    uint64_t checksum = 1469598103934665603ULL;
    int failures = 0;

    /* Every 32-bit value with a single bit set, its neighbours, and a spread of
     * multi-bit patterns: exhaustive over the interesting low range, then a
     * multiplicative sweep for the rest. */
    for (unsigned i = 0; i < 32; i++) {
        uint32_t x = 1U << i;
        uint32_t probes[5];
        probes[0] = x;
        probes[1] = x - 1U;
        probes[2] = x + 1U;
        probes[3] = x | (x >> 3);
        probes[4] = ~x;
        for (unsigned k = 0; k < 5; k++) {
            uint32_t v = probes[k];
            unsigned want_clz = ref_clz32(v | 1U);
            unsigned want_ctz = ref_ctz32(v | 0x80000000U);
            if (clz_guarded(v) != want_clz) {
                printf("clz_guarded(%08x) = %u, want %u\n", v, clz_guarded(v), want_clz);
                failures++;
            }
            if (ctz_guarded(v) != want_ctz) {
                printf("ctz_guarded(%08x) = %u, want %u\n", v, ctz_guarded(v), want_ctz);
                failures++;
            }
            /* 32 - clz(v|1) == bsr(v|1) + 1 */
            unsigned want_bb = 32U - want_clz;
            if (byte_bit_length(v) != (uint8_t)want_bb) {
                printf("byte_bit_length(%08x) = %u, want %u\n", v, byte_bit_length(v),
                       (uint8_t)want_bb);
                failures++;
            }
            unsigned a = ref_ctz32(v | 0x80000000U);
            if (cap_trailing(v) != (a < 6 ? a : 6)) {
                printf("cap_trailing(%08x) mismatch\n", v);
                failures++;
            }
            unsigned b = ref_clz32(v | 1U);
            if (cap_leading(v) != (b < 6 ? b : 6)) {
                printf("cap_leading(%08x) mismatch\n", v);
                failures++;
            }
            checksum = (checksum ^ (uint64_t)(want_clz + want_ctz)) * 1099511628211ULL;
        }
    }

    /* v == 0: the case the guards exist for. */
    /* clz(0|1) = 31, ctz(0|0x80000000) = 31, and 1 + (31 ^ 31) = 1. */
    if (clz_guarded(0) != 31U || ctz_guarded(0) != 31U || byte_bit_length(0) != 1U) {
        printf("zero-input guard results are wrong\n");
        failures++;
    }

    /* Mixed operand pairs. */
    for (unsigned i = 0; i < 4096; i++) {
        uint32_t x = i * 2654435761U;
        uint32_t y = (i ^ 0x9E3779B9U) * 40503U;
        unsigned a = ref_ctz32(x | 0x80000000U);
        unsigned b = ref_clz32(y | 1U);
        if (mixed_min(x, y) != (a < b ? a : b)) {
            printf("mixed_min(%08x,%08x) mismatch\n", x, y);
            failures++;
        }
        /* Nonzero operands only: the plain forms are undefined at zero. */
        if (x != 0 && (clz_plain(x) != ref_clz32(x) || ctz_plain(x) != ref_ctz32(x))) {
            printf("plain counts disagree at %08x\n", x);
            failures++;
        }
        checksum = (checksum ^ (uint64_t)(a + b)) * 1099511628211ULL;
    }

    /* 64-bit guard, including the zero input. */
    for (unsigned i = 0; i < 64; i++) {
        uint64_t v = 1ULL << i;
        if (clz64_guarded(v) != (unsigned)__builtin_clzll(v) ||
            clz64_guarded(v - 1) != (v == 1 ? 63U : (unsigned)__builtin_clzll(v - 1)) ||
            clz64_guarded(~v) != (i == 63 ? 1U : 0U)) {
            printf("clz64_guarded disagrees at bit %u\n", i);
            failures++;
        }
    }
    if (clz64_guarded(0) != 63U) {
        printf("clz64_guarded(0) = %u, want 63\n", clz64_guarded(0));
        failures++;
    }

    if (failures) {
        printf("%d bit-count failure(s)\n", failures);
        return 1;
    }
    printf("clz_zero_guard checksum=%llu\n", (unsigned long long)checksum);
    return 0;
}
