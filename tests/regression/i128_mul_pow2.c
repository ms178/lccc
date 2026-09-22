/*
 * i128_mul_pow2.c -- 128-bit multiply by powers of two, the SIBLING consumer of
 * the widened `const_power_of_two` predicate.
 *
 * `const_power_of_two` feeds exactly two strength reductions:
 *
 *   x / 2^k  =>  x >> k          (UDiv,  a shift AMOUNT)
 *   x % 2^k  =>  x & (2^k - 1)   (URem,  a MASK)
 *   x * 2^k  =>  x << k          (Mul,   a shift AMOUNT)
 *
 * PR #595 widened the predicate's range to 2^127 without widening the one
 * consumer that builds a mask, and `x % 2^k` miscompiled for k >= 64 (see
 * i128_divrem_pow2.c).  This file covers the OTHER consumer, which carries a
 * shift amount rather than a mask and so was not affected -- the distinction
 * is exactly what the fix turns on, and it deserves a permanent guard rather
 * than a one-off verification.
 *
 * A shift amount is representable for every k the predicate can report
 * (k <= 127 fits an i64, and `const_power_of_two` masks to the type width so k
 * is always narrower than the type), so this path should never have been
 * wrong -- which is the point of testing it: if the two consumers ever get
 * conflated, or a mask construction is introduced here, this file fails.
 *
 * Coverage mirrors i128_divrem_pow2.c: literal divisors only (a runtime
 * divisor never reaches the fold), the 63/64 boundary, both 128-bit
 * signednesses, and controls at non-powers-of-two.
 */

#include <stdio.h>

typedef unsigned __int128 u128;

/* One noinline wrapper per factor so the factor stays a literal. */
#define GEN(K)                                                          \
    __attribute__((noinline)) static u128 mul_pow##K(u128 x)            \
    {                                                                   \
        return x * (((u128)1) << (K));                                  \
    }

GEN(1) GEN(2) GEN(8) GEN(31) GEN(32) GEN(62) GEN(63) GEN(64) GEN(65) GEN(96) \
GEN(100) GEN(126) GEN(127)

/* Not powers of two: the fold must not fire.  The reference for these is the
 * defining shift identity, so a mismatch means the fold fired wrongly. */
#define GEN_CTRL(NAME, F)                                               \
    __attribute__((noinline)) static u128 NAME(u128 x) { return x * (F); }

GEN_CTRL(mul_3, 3)
GEN_CTRL(mul_6, 6)
GEN_CTRL(mul_pow63_plus1, (((u128)1) << 63) + 1)
GEN_CTRL(mul_pow64_plus1, (((u128)1) << 64) + 1)

static unsigned long long bad;
static const char *what;

static void check(const char *name, u128 got, u128 want)
{
    if (got != want) {
        bad++;
        if (bad == 1)
            what = name;
    }
}

#define C(K, X) check("mul 2^" #K, mul_pow##K(X), (X) << (K))

int main(void)
{
    /* Values that place set bits on both sides of every boundary above, and
     * the extremes.  The modular identities below are the oracle: for a
     * power-of-two factor the product is exact, and for the non-power-of-two
     * controls the product is checked against repeated shifting, which is an
     * independent formulation. */
    static const u128 cases[] = {
        0,
        1,
        2,
        3,
        (u128)1 << 31,
        ((u128)1 << 32) - 1,
        (u128)1 << 32,
        ((u128)1 << 63) - 1,
        (u128)1 << 63,
        ((u128)0x0123456789ABCDEFULL << 64) | (u128)0xFEDCBA9876543210ULL,
        (u128)1 << 64,
        ((u128)1 << 100) - 1,
        (u128)1 << 126,
        (u128)1 << 127,
    };
    unsigned long long i;

    for (i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        u128 x = cases[i];

        C(1, x);
        C(2, x);
        C(8, x);
        C(31, x);
        C(32, x);
        C(62, x);
        C(63, x);
        C(64, x);
        C(65, x);
        C(96, x);
        C(100, x);
        C(126, x);
        C(127, x);

        /* Controls, referenced by repeated doubling instead of a shift. */
        check("mul 3", mul_3(x), x + x + x);
        check("mul 6", mul_6(x), (x + x + x) + (x + x + x));
        check("mul 2^63+1", mul_pow63_plus1(x),
              (x << 63) + x);
        check("mul 2^64+1", mul_pow64_plus1(x),
              (x << 64) + x);
    }

    if (bad) {
        printf("FAILED case=%s (%llu bad)\n", what, bad);
        return 1;
    }
    printf("ALL-OK (0 bad)\n");
    return 0;
}
