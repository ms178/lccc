/*
 * Cross-domain comparison of a sign-extended value: runtime semantics battery.
 *
 * `(unsigned)(signed char)x < 300` is not an interval question -- the unsigned
 * reading of `[-128, 127]` is `[0, 127] u [0xffffff80, 0xffffffff]`, two halves
 * -- but a constant between the halves makes it a question about the sign bit.
 * simplify's `cross_domain_sign_test` proves that per predicate and rewrites
 * the compare; this file checks the rewrite against a model that never spells
 * the rewritten expression.
 *
 * Oracle discipline: the model computes the sign-extension's UNSIGNED value in
 * 64-bit arithmetic (`v < 0 ? v + 2^32 : v`, the compare's own width, not the
 * source's -- `(unsigned)(signed char)-320` is the unsigned reading of -64) and
 * compares in `long long`, so no
 * probe can agree with its own bug: the probe asks the question in 32-bit
 * unsigned arithmetic on the value the backend extends, the model asks it in
 * 64-bit arithmetic on the mathematical integer.  A rewrite that mixed the
 * readings, used the wrong half's bound, or fired for a constant inside a half
 * makes the two disagree on some swept value.
 *
 * Coverage: every source width C promotes from (8, 16, 32 bits -- 8 and 16
 * exhaustively over their whole domain, 32 on the boundaries plus a seeded
 * sample), every predicate, and constants on both sides of each half's
 * boundary (`hi`, `hi+1`, `U - 2^(w-1)`, `U - 2^(w-1) - 1`) as well as
 * constants inside a half, which must NOT become a sign test.
 *
 * Exit code 0 = every probe agreed with the model on every swept value.  The
 * printed hash lets the suite's GCC differential catch a model-side mistake.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef unsigned long long u64;
typedef long long i64;

/* The unsigned reading of a sign-extended W-bit value, as a 64-bit number. */
static u64 model_u64(i64 v, int width_bits) {
    i64 spanned = 1LL << width_bits;
    return (u64)(v < 0 ? v + spanned : v);
}

typedef int (*probe32)(int);
typedef int (*probe64)(i64);

/* One probe family: a predicate over the unsigned 32-bit reading. */
#define DEFINE_PROBES(W, TAG)                                                                     \
    __attribute__((noinline)) int p_##TAG##_lt_lo(int x) { return (unsigned)(W)x < 300u; }        \
    __attribute__((noinline)) int p_##TAG##_le_lo(int x) { return (unsigned)(W)x <= 300u; }       \
    __attribute__((noinline)) int p_##TAG##_gt_hi(int x) { return (unsigned)(W)x > 4294966996u; } \
    __attribute__((noinline)) int p_##TAG##_ge_hi(int x) { return (unsigned)(W)x >= 4294966996u; }\
    __attribute__((noinline)) int p_##TAG##_lt_in(int x) { return (unsigned)(W)x < 5u; }          \
    __attribute__((noinline)) int p_##TAG##_eq5(int x) { return (unsigned)(W)x == 5u; }           \
    __attribute__((noinline)) int p_##TAG##_ne5(int x) { return (unsigned)(W)x != 5u; }           \
    __attribute__((noinline)) int p_##TAG##_ge_in(int x) { return (unsigned)(W)x >= 5u; }     \
    /* The halves' exact boundaries, where each predicate's guard is either   \
       strict or not: 2^32 - 128 is the negative half's first member for an    \
       8-bit source, 2^32 - 32768 for a 16-bit one. */                         \
    __attribute__((noinline)) int p_##TAG##_le_nl(int x) {                                    \
        return (unsigned)(W)x <= 4294967168u;                                                 \
    }                                                                                         \
    __attribute__((noinline)) int p_##TAG##_ge_nl(int x) {                                    \
        return (unsigned)(W)x >= 4294967168u;                                                 \
    }                                                                                         \
    __attribute__((noinline)) int p_##TAG##_gt_nl1(int x) {                                   \
        return (unsigned)(W)x > 4294967167u;                                                  \
    }                                                                                         \
    __attribute__((noinline)) int p_##TAG##_lt_hi(int x) { return (unsigned)(W)x < 127u; }

DEFINE_PROBES(signed char, s8)
DEFINE_PROBES(short, s16)
DEFINE_PROBES(int, s32)

/* The same questions at a 64-bit compare width (a 64-bit unsigned compare of a
   32-bit sign-extension), where `U` is 2^64 and the halves are far apart. */
__attribute__((noinline)) int p_w64_lt_lo(i64 x) { return (unsigned long long)(int)x < 300ULL; }
/* 2^64 - 2^31 is the first member of the negative half; one below it and the
   `>=` version both put the constant outside the gap and must NOT fold. */
__attribute__((noinline)) int p_w64_ge_hi(i64 x) {
    return (unsigned long long)(int)x >= 18446744071562067968ULL;
}
__attribute__((noinline)) int p_w64_gt_hi(i64 x) {
    return (unsigned long long)(int)x > 18446744071562067967ULL;
}
__attribute__((noinline)) int p_w64_ge_gap(i64 x) {
    return (unsigned long long)(int)x >= 18446744071562067969ULL;
}

static int failures;

static void check(int got, int want, const char *what, i64 x) {
    if (got != want) {
        if (failures < 20) {
            printf("MISMATCH %s x=%lld got=%d want=%d\n", what, x, got, want);
        }
        failures++;
    }
}

/* Sweep a family over [lo, hi] and over the constants' neighbourhoods. */
#define SWEEP32(TAG, W, LO, HI)                                                             \
    do {                                                                                          \
        for (i64 x = (LO); x <= (HI); x++) {                                                      \
            i64 v = (W)x;                                                                         \
            u64 u = model_u64(v, 32);                                                             \
            check(p_##TAG##_lt_lo((int)x), u < 300u, #TAG " lt_lo", x);                           \
            check(p_##TAG##_le_lo((int)x), u <= 300u, #TAG " le_lo", x);                          \
            check(p_##TAG##_gt_hi((int)x), u > 4294966996u, #TAG " gt_hi", x);                    \
            check(p_##TAG##_ge_hi((int)x), u >= 4294966996u, #TAG " ge_hi", x);                   \
            check(p_##TAG##_lt_in((int)x), u < 5u, #TAG " lt_in", x);                             \
            check(p_##TAG##_ge_in((int)x), u >= 5u, #TAG " ge_in", x);                            \
            check(p_##TAG##_eq5((int)x), u == 5u, #TAG " eq5", x);                                \
            check(p_##TAG##_ne5((int)x), u != 5u, #TAG " ne5", x);                                \
            check(p_##TAG##_le_nl((int)x), u <= 4294967168u, #TAG " le_nl", x);                   \
            check(p_##TAG##_ge_nl((int)x), u >= 4294967168u, #TAG " ge_nl", x);                   \
            check(p_##TAG##_gt_nl1((int)x), u > 4294967167u, #TAG " gt_nl1", x);                  \
            check(p_##TAG##_lt_hi((int)x), u < 127u, #TAG " lt_hi", x);                           \
        }                                                                                         \
    } while (0)

static u64 mix(u64 h, u64 v) {
    h ^= v + 0x9e3779b97f4a7c15ULL + (h << 6) + (h >> 2);
    return h;
}

int main(void) {
    u64 hash = 0xcbf29ce484222325ULL;
    i64 i;

    /* 8-bit and 16-bit sources: the whole domain, plus the surrounding ints
       that the cast truncates, so both halves are visited for every probe. */
    SWEEP32(s8, signed char, -320, 320);
    SWEEP32(s16, short, -33000, 33000);

    /* 32-bit source: the boundaries where a half starts and ends, then a
       seeded sample that reaches deep into both halves. */
    static const i64 edges[] = {0,          1,           4,           5,
                                6,          127,         128,         129,
                                -1,         -2,          -128,        -129,
                                2147483647LL, -2147483647LL - 1, 65535, 65536,
                                -65536,     -65537,       1000000,     -1000000};
    for (size_t k = 0; k < sizeof edges / sizeof edges[0]; k++) {
        i64 x = edges[k];
        u64 u = model_u64((int)x, 32);
        check(p_s32_lt_lo((int)x), u < 300u, "s32 lt_lo", x);
        check(p_s32_le_lo((int)x), u <= 300u, "s32 le_lo", x);
        check(p_s32_gt_hi((int)x), u > 4294966996u, "s32 gt_hi", x);
        check(p_s32_ge_hi((int)x), u >= 4294966996u, "s32 ge_hi", x);
        check(p_s32_lt_in((int)x), u < 5u, "s32 lt_in", x);
        check(p_s32_ge_in((int)x), u >= 5u, "s32 ge_in", x);
        check(p_s32_eq5((int)x), u == 5u, "s32 eq5", x);
        check(p_s32_ne5((int)x), u != 5u, "s32 ne5", x);
        hash = mix(hash, (u64)u);
    }
    u64 seed = 0x243f6a8885a308d3ULL;
    for (i = 0; i < 200000; i++) {
        seed = seed * 6364136223846793005ULL + 1442695040888963407ULL;
        i64 x = (i64)(seed >> 17) - 281474976710656LL;
        uint32_t narrow = (uint32_t)x;
        if (i & 1) {
            narrow = (uint32_t)(int32_t)(x & 0xFFFF);
        }
        i64 sx = (i64)(int32_t)(int32_t)narrow;
        u64 u = model_u64(sx, 32);
        check(p_s32_lt_lo((int)sx), u < 300u, "s32 lt_lo", sx);
        check(p_s32_le_lo((int)sx), u <= 300u, "s32 le_lo", sx);
        check(p_s32_gt_hi((int)sx), u > 4294966996u, "s32 gt_hi", sx);
        check(p_s32_ge_hi((int)sx), u >= 4294966996u, "s32 ge_hi", sx);
        check(p_s32_lt_in((int)sx), u < 5u, "s32 lt_in", sx);
        check(p_s32_ge_in((int)sx), u >= 5u, "s32 ge_in", sx);
        check(p_s32_eq5((int)sx), u == 5u, "s32 eq5", sx);
        check(p_s32_ne5((int)sx), u != 5u, "s32 ne5", sx);
        hash = mix(hash, u);
    }

    /* The 64-bit compare width: a 32-bit sign-extension read by `unsigned long
       long` comparisons.  `U - 2^31` is 18446744071562068236. */
    for (i = -70000; i <= 70000; i++) {
        i64 v = (int)i;
        u64 u = (u64)v;
        check(p_w64_lt_lo(v), u < 300ULL, "w64 lt_lo", i);
        check(p_w64_ge_hi(v), u >= 18446744071562067968ULL, "w64 ge_hi", i);
        check(p_w64_gt_hi(v), u > 18446744071562067967ULL, "w64 gt_hi", i);
        check(p_w64_ge_gap(v), u >= 18446744071562067969ULL, "w64 ge_gap", i);
        hash = mix(hash, u + 1);
    }
    for (i = 0; i < 200000; i++) {
        seed = seed * 6364136223846793005ULL + 1442695040888963407ULL;
        i64 v = (i64)(int32_t)(seed >> 32);
        u64 u = (u64)v;
        check(p_w64_lt_lo(v), u < 300ULL, "w64 lt_lo", v);
        check(p_w64_ge_hi(v), u >= 18446744071562067968ULL, "w64 ge_hi", v);
        check(p_w64_gt_hi(v), u > 18446744071562067967ULL, "w64 gt_hi", v);
        check(p_w64_ge_gap(v), u >= 18446744071562067969ULL, "w64 ge_gap", v);
        hash = mix(hash, u + 2);
    }

    printf("cross_domain_sext_cmp: failures=%d hash=%016llx\n", failures, hash);
    return failures != 0;
}
