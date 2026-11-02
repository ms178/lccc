/*
 * i128_divrem_pow2.c -- 128-bit divide/remainder by powers of two.
 *
 * Guards the mask the `x % 2^k  =>  x & (2^k - 1)` strength reduction builds.
 *
 * `const_power_of_two` reports shifts up to 127 for the 128-bit types.  The
 * mask used to be constructed in i64:
 *
 *     let mask = (1i64 << shift) - 1;
 *
 * which is wrong twice over once shift can reach 128-bit territory:
 *
 *   release (overflow-checks off): `1i64 << 64` is `1i64` -- the shift amount
 *     is masked modulo 64 -- so the mask for 2^64 came out 0 and `x % 2^64`
 *     compiled to `x & 0`.  Every larger k wrapped to `2^(k mod 64) - 1`, so
 *     `x % 2^100` masked with only the low 36 bits.
 *
 *   debug (overflow-checks on): `(1i64 << 63) - 1` panics outright, so even a
 *     u64 remainder by 2^63 aborted the compiler.
 *
 * The release form was a SILENT wrong-code miscompile: the program assembled,
 * ran and printed plausible values.  The mask is now built at the operation's
 * width by `IrConst::low_mask` (u128 accumulator), so this file fails loudly
 * against the old compiler and passes against the new one.
 *
 * Shape notes, each of which was measured rather than assumed:
 *
 *  - The divisor must be a LITERAL.  A divisor read from an array or computed
 *    into a variable is a runtime value, `const_power_of_two` never fires, and
 *    the test would validate the libcall path instead of the fold.  Each
 *    divisor therefore gets its own noinline function.
 *  - `1 << k` written as a C constant expression is folded to an IR constant
 *    before the strength-reduction pass runs, which is what makes the fold
 *    reachable here; verified by the old binary failing this file at both -O1
 *    and -O2.
 *  - `__int128` has no printf conversion in ISO C, so results are compared in
 *    the program and only the verdict is printed.
 */

#include <stdio.h>

typedef unsigned __int128 u128;
typedef signed __int128 s128;

/* One noinline wrapper per divisor so the divisor stays a literal. */
#define GEN_REM(K)                                                     \
    __attribute__((noinline)) static u128 urem_pow##K(u128 x)          \
    {                                                                  \
        return x % (((u128)1) << (K));                                 \
    }
#define GEN_QUO(K)                                                     \
    __attribute__((noinline)) static u128 udiv_pow##K(u128 x)          \
    {                                                                  \
        return x / (((u128)1) << (K));                                 \
    }

/* The boundary set: below the i64 limit, exactly at it, one past it, and up to
 * the largest power of two a u128 can hold.  63 and 64 are the pair that the
 * i64 construction got exactly backwards (63 correct-by-wrap in release and a
 * panic in debug; 64 wholly wrong in release). */
GEN_REM(1) GEN_QUO(1)
GEN_REM(31) GEN_QUO(31)
GEN_REM(32) GEN_QUO(32)
GEN_REM(60) GEN_QUO(60)
GEN_REM(62) GEN_QUO(62)
GEN_REM(63) GEN_QUO(63)
GEN_REM(64) GEN_QUO(64)
GEN_REM(65) GEN_QUO(65)
GEN_REM(96) GEN_QUO(96)
GEN_REM(100) GEN_QUO(100)
GEN_REM(126) GEN_QUO(126)
GEN_REM(127) GEN_QUO(127)

/* Controls: not powers of two, so the fold must NOT fire and the libcall path
 * must stay correct.  If one of these fails, the defect is something else. */
#define GEN_CTRL(NAME, D)                                              \
    __attribute__((noinline)) static u128 NAME(u128 x) { return x % (D); }
GEN_CTRL(urem_3, 3)
GEN_CTRL(urem_pow63_plus1, (((u128)1) << 63) + 1)
GEN_CTRL(urem_pow64_plus1, (((u128)1) << 64) + 1)
GEN_CTRL(urem_allones, ~(u128)0)

/* Signed 128-bit remainder: SRem is deliberately not strength-reduced (the
 * sign fixup is a separate sequence), so this is a control that the widening
 * did not accidentally capture it. */
__attribute__((noinline)) static s128 srem_pow64(s128 x)
{
    return x % (s128)(((u128)1) << 64);
}
__attribute__((noinline)) static s128 srem_pow63(s128 x)
{
    return x % (s128)(((u128)1) << 63);
}

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

#define CHECK_REM(K, X) check("urem 2^" #K, urem_pow##K(X), (X) & (mask_k(K)))
#define CHECK_QUO(K, X) check("udiv 2^" #K, udiv_pow##K(X), (X) >> (K))

static u128 mask_k(int k)
{
    return k >= 128 ? ~(u128)0 : ((((u128)1) << k) - 1);
}

/* Dividends that straddle every bit position the divisors above can split:
 * values below, at and above each boundary, the all-ones word, a pattern with
 * both halves non-zero, and the exact multiples of the divisors. */
static const u128 cases[] = {
    0,
    1,
    2,
    3,
    ((u128)1 << 31) - 1,
    (u128)1 << 31,
    ((u128)1 << 32) - 1,
    (u128)1 << 32,
    ((u128)1 << 60) - 1,
    (u128)1 << 60,
    ((u128)1 << 63) - 1,
    (u128)1 << 63,
    (((u128)1 << 63) - 1) | ((u128)1 << 64),
    (u128)1 << 64,
    ((u128)1 << 64) + 1,
    ((u128)0x123456789ABCDEF0ULL << 64) | (u128)0xFEDCBA9876543210ULL,
    ((u128)1 << 100) - 1,
    (u128)1 << 100,
    ((u128)1 << 126) - 1,
    (u128)1 << 126,
    ((u128)1 << 127) - 1,
    (u128)1 << 127,
    ~(u128)0,
    ~(u128)0 - 1,
};

int main(void)
{
    unsigned long long i;

    for (i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        u128 x = cases[i];

        CHECK_REM(1, x);
        CHECK_REM(31, x);
        CHECK_REM(32, x);
        CHECK_REM(60, x);
        CHECK_REM(62, x);
        CHECK_REM(63, x);
        CHECK_REM(64, x);
        CHECK_REM(65, x);
        CHECK_REM(96, x);
        CHECK_REM(100, x);
        CHECK_REM(126, x);
        CHECK_REM(127, x);

        CHECK_QUO(1, x);
        CHECK_QUO(31, x);
        CHECK_QUO(32, x);
        CHECK_QUO(60, x);
        CHECK_QUO(62, x);
        CHECK_QUO(63, x);
        CHECK_QUO(64, x);
        CHECK_QUO(65, x);
        CHECK_QUO(96, x);
        CHECK_QUO(100, x);
        CHECK_QUO(126, x);
        CHECK_QUO(127, x);

        /* Controls: the reference here is an independent libcall, so a
         * mismatch means the fold fired where it should not have. */
        check("urem 3", urem_3(x), x - (u128)3 * (x / (u128)3));
        check("urem 2^63+1", urem_pow63_plus1(x), x - ((((u128)1 << 63) + 1) * (x / (((u128)1 << 63) + 1))));
        check("urem 2^64+1", urem_pow64_plus1(x), x - ((((u128)1 << 64) + 1) * (x / (((u128)1 << 64) + 1))));
        /* Divisor 2^128-1 has a closed form independent of any division:
         * every x < 2^128-1 is already its own remainder, and the single
         * input that reaches the divisor maps to 0. */
        check("urem all-ones", urem_allones(x),
              (x == ~(u128)0) ? (u128)0 : x);
    }

    /* Signed: result must match a sign-corrected reference. */
    {
        s128 v = (s128)(((u128)1 << 127) | (u128)0x5A5A5A5A5A5A5A5AULL);
        check("srem 2^64 (neg)", (u128)srem_pow64(-v), (u128)((-v) % ((s128)(((u128)1) << 64))));
        check("srem 2^63 (neg)", (u128)srem_pow63(-v), (u128)((-v) % ((s128)(((u128)1) << 63))));
        check("srem 2^64 (pos)", (u128)srem_pow64(v), (u128)(v % ((s128)(((u128)1) << 64))));
    }

    if (bad) {
        printf("FAILED case=%s (%llu bad)\n", what, bad);
        return 1;
    }
    printf("ALL-OK (0 bad)\n");
    return 0;
}
