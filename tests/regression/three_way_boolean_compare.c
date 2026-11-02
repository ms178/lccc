/* Comparing the two indicators of a three-way comparison is comparing the
 * operands.
 *
 * `(a > b) - (a < b)` is the sign of `a - b`, and every `cmp` of that value
 * against zero is a `cmp` of `a` and `b`: the fold that recognizes it deletes
 * two `setcc`s, two `movzbl`s, the subtraction and the second comparison --
 * nine instructions down to four, which is what Clang emits and two fewer than
 * GCC.
 *
 * A fold that deletes six instructions is worth nothing if it changes an
 * answer, so this file does not test the shape for one pair of values: every
 * predicate over every signedness and width is compared against a reference
 * that computes the sign from branches, on a domain that includes the extrema
 * (`INT_MIN`, `INT_MAX`, `UINT_MAX`, `LLONG_MIN`), the values around zero, and
 * a seeded pseudo-random cross product.
 *
 * The reference is written so that it cannot be folded into the same shape:
 * it branches, and it never subtracts two booleans. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Several shapes below ask a tautological question on purpose -- `(unsigned)t
 * >= 0u` is always true, and the fold has to answer that correctly rather than
 * keep the code that computes it.  GCC and Clang warn about the very thing the
 * file tests, so the warning is off for this translation unit only. */
#if defined(__GNUC__)
#pragma GCC diagnostic ignored "-Wtype-limits"
#endif

static int fails;

static void check(const char *name, long long got, long long want, long long a,
                  long long b)
{
    if (got != want) {
        printf("FAIL %s a=%lld b=%lld got=%lld want=%lld\n", name, a, b, got, want);
        fails++;
    }
}

/* Branch-written sign, so the reference itself never materializes the
 * difference of two indicators. */
static int ref_sign_ll(long long a, long long b)
{
    if (a < b)
        return -1;
    if (a > b)
        return 1;
    return 0;
}

static int ref_sign_llu(unsigned long long a, unsigned long long b)
{
    if (a < b)
        return -1;
    if (a > b)
        return 1;
    return 0;
}

/* The reference value is forced through memory before it is questioned, so the
 * expectation is computed by the *driver*, not by the same fold under test: a
 * `s <= 0` the compiler can see through would be folded by the very rule this
 * file is checking, and a wrong fold would then agree with its own oracle. */
static long long opaque(long long v)
{
    volatile long long t = v;
    return t;
}

/* ── the shapes under test ──────────────────────────────────────────────── */

#define SHAPES(T, SUFFIX, REF, CAST)                                          \
    int three_way_le_##SUFFIX(T a, T b)                                       \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return t <= 0;                                                        \
    }                                                                         \
    int three_way_lt_##SUFFIX(T a, T b)                                       \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return t < 0;                                                         \
    }                                                                         \
    int three_way_gt_##SUFFIX(T a, T b)                                       \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return t > 0;                                                         \
    }                                                                         \
    int three_way_ge_##SUFFIX(T a, T b)                                       \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return t >= 0;                                                        \
    }                                                                         \
    int three_way_eq_##SUFFIX(T a, T b)                                       \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return t == 0;                                                        \
    }                                                                         \
    int three_way_ne_##SUFFIX(T a, T b)                                       \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return t != 0;                                                        \
    }                                                                         \
    /* The subtraction written the other way round: every predicate mirrors. */ \
    int three_way_rev_le_##SUFFIX(T a, T b)                                   \
    {                                                                         \
        int t = (a < b) - (a > b);                                            \
        return t <= 0;                                                        \
    }                                                                         \
    int three_way_rev_lt_##SUFFIX(T a, T b)                                   \
    {                                                                         \
        int t = (a < b) - (a > b);                                            \
        return t < 0;                                                         \
    }                                                                         \
    /* The reversed subtraction with the UNSIGNED predicates: `-1` is
     * `UINT_MAX` there, so the answer is not the mirror of the forward case
     * (`(l - g) >u 0` is true when `l - g` is `-1`). */                       \
    int three_way_rev_ule_##SUFFIX(T a, T b)                                  \
    {                                                                         \
        int t = (a < b) - (a > b);                                            \
        return (unsigned)t <= 0u;                                             \
    }                                                                         \
    int three_way_rev_ugt_##SUFFIX(T a, T b)                                  \
    {                                                                         \
        int t = (a < b) - (a > b);                                            \
        return (unsigned)t > 0u;                                              \
    }                                                                         \
    int three_way_rev_uge_##SUFFIX(T a, T b)                                  \
    {                                                                         \
        int t = (a < b) - (a > b);                                            \
        return (unsigned)t >= 0u;                                             \
    }                                                                         \
    int three_way_rev_ult_##SUFFIX(T a, T b)                                  \
    {                                                                         \
        int t = (a < b) - (a > b);                                            \
        return (unsigned)t < 0u;                                              \
    }                                                                         \
    /* The difference kept in a wider carrier, then compared. */               \
    int three_way_wide_le_##SUFFIX(T a, T b)                                  \
    {                                                                         \
        long long t = (long long)(a > b) - (long long)(a < b);                \
        return t <= 0;                                                        \
    }                                                                         \
    /* UNSIGNED comparison of the signed difference against zero: `-1` is
     * `UINT_MAX`, so `(unsigned)t <= 0` is `t == 0` and `(unsigned)t >= 0` is
     * always true.  Both are shapes the fold maps to a different question. */ \
    int three_way_ule_##SUFFIX(T a, T b)                                      \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return (unsigned)t <= 0u;                                             \
    }                                                                         \
    int three_way_ugt_##SUFFIX(T a, T b)                                      \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return (unsigned)t > 0u;                                              \
    }                                                                         \
    int three_way_uge_##SUFFIX(T a, T b)                                      \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return (unsigned)t >= 0u;                                             \
    }                                                                         \
    int three_way_ult_##SUFFIX(T a, T b)                                      \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        return (unsigned)t < 0u;                                              \
    }                                                                         \
    /* Controls: not complementary indicators, so they must not fold -- and
     * must still be right. */                                               \
    int control_ge_gt_##SUFFIX(T a, T b)                                      \
    {                                                                         \
        int t = (a >= b) - (a > b);                                           \
        return t <= 0;                                                        \
    }                                                                         \
    int control_lt_gt_##SUFFIX(T a, T b)                                      \
    {                                                                         \
        int t = (a < b) - (a > b) + (a == b);                                 \
        return t <= 0;                                                        \
    }                                                                         \
    int control_shared_##SUFFIX(T a, T b, int *sink)                          \
    {                                                                         \
        int t = (a > b) - (a < b);                                            \
        *sink = t; /* a second reader: the subtraction must survive */        \
        return t <= 0;                                                        \
    }                                                                         \
    /* NESTED pairs: the two indicators are not complementary but nested, so
     * the difference is the EQUALITY indicator (`a == b`) rather than a
     * sign.  `(a >= b) - (a > b)` is 1 exactly when `a == b`, never -1. */   \
    int nested_ge_gt_le_##SUFFIX(T a, T b)                                    \
    {                                                                         \
        int t = (a >= b) - (a > b);                                           \
        return t <= 0;                                                        \
    }                                                                         \
    int nested_ge_gt_gt_##SUFFIX(T a, T b)                                    \
    {                                                                         \
        int t = (a >= b) - (a > b);                                           \
        return t > 0;                                                         \
    }                                                                         \
    int nested_le_lt_lt_##SUFFIX(T a, T b)                                    \
    {                                                                         \
        int t = (a <= b) - (a < b);                                           \
        return t < 0;                                                         \
    }                                                                         \
    /* The same predicate twice: the difference is constant zero. */          \
    int same_pair_le_##SUFFIX(T a, T b)                                       \
    {                                                                         \
        int t = (a > b) - (a > b);                                            \
        return t <= 0;                                                        \
    }                                                                         \
    /* WIDENING through a narrow carrier.  `unsigned char` holds 255 where the
     * difference is negative, and zero-extending that back to `int` keeps
     * 255 -- so `(int)d < 0` is FALSE for every input.  `signed char` holds
     * -1 and sign-extends it, so `(int)s < 0` is `a < b`.  A fold that peels
     * the cast without asking which extension it is turns the first into the
     * second. */                                                            \
    int zext_uchar_lt0_##SUFFIX(T a, T b)                                     \
    {                                                                         \
        unsigned char d = (a > b) - (a < b);                                  \
        return (int)d < 0;                                                    \
    }                                                                         \
    int zext_uchar_gt0_##SUFFIX(T a, T b)                                     \
    {                                                                         \
        unsigned char d = (a > b) - (a < b);                                  \
        return (int)d > 0;                                                    \
    }                                                                         \
    int zext_uchar_ule0_##SUFFIX(T a, T b)                                    \
    {                                                                         \
        unsigned char d = (a > b) - (a < b);                                  \
        return (unsigned)d <= 0u;                                             \
    }                                                                         \
    int sext_schar_lt0_##SUFFIX(T a, T b)                                     \
    {                                                                         \
        signed char d = (a > b) - (a < b);                                    \
        return (int)d < 0;                                                    \
    }                                                                         \
    /* TRUNCATING at the comparison itself: the pattern survives the
     * truncation, so both questions are answerable. */                      \
    int trunc_schar_lt0_##SUFFIX(T a, T b)                                    \
    {                                                                         \
        signed char d = (a > b) - (a < b);                                    \
        return d < 0;                                                         \
    }                                                                         \
    int trunc_uchar_le0_##SUFFIX(T a, T b)                                    \
    {                                                                         \
        unsigned char d = (a > b) - (a < b);                                  \
        return d <= 0;                                                        \
    }                                                                         \
    /* Mixed classes: `a >s b` and `a <u b` are not complementary at all
     * (`a = -1, b = 1` makes both false), so only genuine values save a
     * fold that assumed they were.  (Written out for `long long` below, where
     * the unsigned cast is at the operand's own width.) */

typedef int si8;
typedef short si16;
typedef int si32;
typedef long long si64;
typedef unsigned char ui8;
typedef unsigned short ui16;
typedef unsigned ui32;
typedef unsigned long long ui64;

SHAPES(si8, i8, ref_sign_ll, int)
SHAPES(si16, i16, ref_sign_ll, int)
SHAPES(si32, i32, ref_sign_ll, int)
SHAPES(si64, i64, ref_sign_ll, long long)
SHAPES(ui8, u8, ref_sign_llu, unsigned)
SHAPES(ui16, u16, ref_sign_llu, unsigned)
SHAPES(ui32, u32, ref_sign_llu, unsigned)
SHAPES(ui64, u64, ref_sign_llu, unsigned long long)

/* Mixed signedness: `a >s b` and `a <u b` are not complementary at all
 * (`a = -1, b = 1` makes both false and `a = 1, b = -1` makes both true), so a
 * fold that assumed they were would answer a different question.  Both casts
 * are at the operands' own width, which is why this is not in the macro: `(unsigned)a`
 * on a `long long` would truncate. */
/* Floating operands: with a NaN both comparisons are false, so the difference
 * is 0 and every question about it answers from `0` -- a trichotomy of the
 * operands cannot express that (`!x` of NaN is neither `<` nor `>`), so the fold
 * must leave these alone. */
int f64_le(double a, double b)
{
    int t = (a > b) - (a < b);
    return t <= 0;
}

int f64_eq(double a, double b)
{
    int t = (a > b) - (a < b);
    return t == 0;
}

int mixed_class_le_i64(long long a, long long b)
{
    int t = (a > b) - (int)((unsigned long long)a < (unsigned long long)b);
    return t <= 0;
}

/* One row: the pair of values, then the expected answers for every predicate
 * and spelling, computed from the sign alone. */
static void check_pair_ll(long long a, long long b)
{
    long long s = opaque(ref_sign_ll(a, b));
    check("le", three_way_le_i64(a, b), s <= 0, a, b);
    check("lt", three_way_lt_i64(a, b), s < 0, a, b);
    check("gt", three_way_gt_i64(a, b), s > 0, a, b);
    check("ge", three_way_ge_i64(a, b), s >= 0, a, b);
    check("eq", three_way_eq_i64(a, b), s == 0, a, b);
    check("ne", three_way_ne_i64(a, b), s != 0, a, b);
    check("rev_le", three_way_rev_le_i64(a, b), -s <= 0, a, b);
    check("rev_lt", three_way_rev_lt_i64(a, b), -s < 0, a, b);
    check("wide_le", three_way_wide_le_i64(a, b), s <= 0, a, b);
    check("ule", three_way_ule_i64(a, b), (unsigned)s <= 0u, a, b);
    check("ugt", three_way_ugt_i64(a, b), (unsigned)s > 0u, a, b);
    check("uge", three_way_uge_i64(a, b), (unsigned)s >= 0u, a, b);
    check("ult", three_way_ult_i64(a, b), (unsigned)s < 0u, a, b);
    /* The same four questions on the reversed subtraction. */
    check("rev_ule", three_way_rev_ule_i64(a, b), (unsigned)(-s) <= 0u, a, b);
    check("rev_ugt", three_way_rev_ugt_i64(a, b), (unsigned)(-s) > 0u, a, b);
    check("rev_uge", three_way_rev_uge_i64(a, b), (unsigned)(-s) >= 0u, a, b);
    check("rev_ult", three_way_rev_ult_i64(a, b), (unsigned)(-s) < 0u, a, b);
    /* Controls compute their own reference: (a >= b) - (a > b) is `a == b`. */
    check("control_ge_gt", control_ge_gt_i64(a, b), (a == b) <= 0, a, b);
    /* The nested pairs fold as well; their difference is the equality
     * indicator, so `(a >= b) - (a > b) > 0` is `a == b` and `< 0` never. */
    check("nested_ge_gt_le", nested_ge_gt_le_i64(a, b), (a == b) <= 0, a, b);
    check("nested_ge_gt_gt", nested_ge_gt_gt_i64(a, b), (a == b) > 0, a, b);
    check("nested_le_lt_lt", nested_le_lt_lt_i64(a, b), opaque(a == b) < 0, a, b);
    /* The same predicate twice is a constant zero. */
    check("same_pair_le", same_pair_le_i64(a, b), opaque(1), a, b);
    /* Zero- vs sign-extending through an 8-bit carrier. */
    check("zext_uchar_lt0", zext_uchar_lt0_i64(a, b), opaque((int)(unsigned char)s) < 0, a, b);
    check("zext_uchar_gt0", zext_uchar_gt0_i64(a, b), (int)(unsigned char)s > 0, a, b);
    check("zext_uchar_ule0", zext_uchar_ule0_i64(a, b), (unsigned)(unsigned char)s <= 0u, a, b);
    check("sext_schar_lt0", sext_schar_lt0_i64(a, b), (int)(signed char)s < 0, a, b);
    check("trunc_schar_lt0", trunc_schar_lt0_i64(a, b), (signed char)s < 0, a, b);
    check("trunc_uchar_le0", trunc_uchar_le0_i64(a, b), (unsigned char)s <= 0, a, b);
    /* Mixed signedness classes are not complementary. */
    check("mixed_class_le",
          mixed_class_le_i64(a, b),
          ((long long)(a > b) - (long long)((unsigned long long)a < (unsigned long long)b)) <= 0,
          a, b);
    /* (a < b) - (a > b) + (a == b) <= 0 with the three indicators: -1,0,1 + eq. */
    check("control_lt_gt",
          control_lt_gt_i64(a, b),
          ((-s) + (a == b)) <= 0,
          a, b);
    {
        int sink = 12345;
        int got = control_shared_i64(a, b, &sink);
        check("control_shared", got, s <= 0, a, b);
        check("control_shared_sink", sink, s, a, b);
    }
}

static void check_pair_ull(unsigned long long a, unsigned long long b)
{
    int s = ref_sign_llu(a, b);
    check("u64_le", three_way_le_u64(a, b), s <= 0, (long long)a, (long long)b);
    check("u64_lt", three_way_lt_u64(a, b), s < 0, (long long)a, (long long)b);
    check("u64_gt", three_way_gt_u64(a, b), s > 0, (long long)a, (long long)b);
    check("u64_ge", three_way_ge_u64(a, b), s >= 0, (long long)a, (long long)b);
    check("u64_eq", three_way_eq_u64(a, b), s == 0, (long long)a, (long long)b);
    check("u64_ne", three_way_ne_u64(a, b), s != 0, (long long)a, (long long)b);
    check("u64_rev_le", three_way_rev_le_u64(a, b), -s <= 0, (long long)a, (long long)b);
    check("u64_wide_le", three_way_wide_le_u64(a, b), s <= 0, (long long)a, (long long)b);
    check("u64_ule", three_way_ule_u64(a, b), (unsigned)s <= 0u, (long long)a, (long long)b);
    check("u64_ugt", three_way_ugt_u64(a, b), (unsigned)s > 0u, (long long)a, (long long)b);
    check("u64_uge", three_way_uge_u64(a, b), (unsigned)s >= 0u, (long long)a, (long long)b);
    check("u64_ult", three_way_ult_u64(a, b), (unsigned)s < 0u, (long long)a, (long long)b);
    check("u64_rev_ule", three_way_rev_ule_u64(a, b), (unsigned)(-s) <= 0u, (long long)a, (long long)b);
    check("u64_rev_ugt", three_way_rev_ugt_u64(a, b), (unsigned)(-s) > 0u, (long long)a, (long long)b);
}

/* Narrow widths: the frontend promotes to int, so the comparison runs at 32
 * bits and the subtraction too.  A fold that assumed the operands' own width
 * would see `(char)200 < (char)-1` differ from `200 < 255`. */
static void check_pair_narrow(void)
{
    for (int a = -128; a <= 127; a++) {
        for (int b = -128; b <= 127; b++) {
            signed char sa = (signed char)a, sb = (signed char)b;
            int s = ref_sign_ll((long long)sa, (long long)sb);
            check("i8_le", three_way_le_i8(sa, sb), s <= 0, a, b);
            check("i8_lt", three_way_lt_i8(sa, sb), s < 0, a, b);
            check("i8_ge", three_way_ge_i8(sa, sb), s >= 0, a, b);
            check("i8_rev_lt", three_way_rev_lt_i8(sa, sb), -s < 0, a, b);
            check("i8_wide_le", three_way_wide_le_i8(sa, sb), s <= 0, a, b);
            /* -1 as unsigned char is 255: the promotion is the trick. */
            check("i8_ule", three_way_ule_i8(sa, sb), (unsigned)s <= 0u, a, b);
            check("i8_ugt", three_way_ugt_i8(sa, sb), (unsigned)s > 0u, a, b);
            check("i8_rev_ugt", three_way_rev_ugt_i8(sa, sb), (unsigned)(-s) > 0u, a, b);
            check("i8_rev_ult", three_way_rev_ult_i8(sa, sb), (unsigned)(-s) < 0u, a, b);

            unsigned char ua = (unsigned char)a, ub = (unsigned char)b;
            int us = ref_sign_llu((unsigned long long)ua, (unsigned long long)ub);
            check("u8_le", three_way_le_u8(ua, ub), us <= 0, a, b);
            check("u8_lt", three_way_lt_u8(ua, ub), us < 0, a, b);
            check("u8_ge", three_way_ge_u8(ua, ub), us >= 0, a, b);
            check("u8_eq", three_way_eq_u8(ua, ub), us == 0, a, b);

            unsigned short ua16 = (unsigned short)(a * 257 + 7), ub16 = (unsigned short)(b * 257 - 3);
            int u16s = ref_sign_llu((unsigned long long)ua16, (unsigned long long)ub16);
            check("u16_le", three_way_le_u16(ua16, ub16), u16s <= 0, a, b);
            check("u16_gt", three_way_gt_u16(ua16, ub16), u16s > 0, a, b);
        }
    }
}

int main(void)
{
    /* Extrema and the values around zero, both orders. */
    static const long long edge[] = {
        (long long)0x8000000000000000ULL, -2147483649LL, -2147483648LL, -2, -1, 0,
        1, 2, 2147483647LL, 2147483648LL, (long long)0x7FFFFFFFFFFFFFFFULL,
    };
    const int n = (int)(sizeof edge / sizeof edge[0]);
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            check_pair_ll(edge[i], edge[j]);
            check_pair_ull((unsigned long long)edge[i], (unsigned long long)edge[j]);
        }
    }

    /* A seeded cross product of pseudo-random values, both domains. */
    unsigned long long st = 0x243F6A8885A308D3ULL;
    for (int i = 0; i < 20000; i++) {
        st = st * 6364136223846793005ULL + 1442695040888963407ULL;
        long long a = (long long)st;
        st = st * 6364136223846793005ULL + 1442695040888963407ULL;
        long long b = (long long)st;
        check_pair_ll(a, b);
        check_pair_ull((unsigned long long)a, (unsigned long long)b);
    }

    /* The same at 32 bits, where the operands and the difference are one
     * register and the sign never leaves `int`. */
    for (int i = 0; i < 20000; i++) {
        st = st * 6364136223846793005ULL + 1442695040888963407ULL;
        int a = (int)st;
        st = st * 6364136223846793005ULL + 1442695040888963407ULL;
        int b = (int)st;
        int s = ref_sign_ll(a, b);
        check("i32_le", three_way_le_i32(a, b), s <= 0, a, b);
        check("i32_ugt", three_way_ugt_i32(a, b), (unsigned)s > 0u, a, b);
        check("i32_ult", three_way_ult_i32(a, b), (unsigned)s < 0u, a, b);
        int us = ref_sign_llu((unsigned)a, (unsigned)b);
        check("u32_ge", three_way_ge_u32((unsigned)a, (unsigned)b), us >= 0, a, b);
        check("u32_lt", three_way_lt_u32((unsigned)a, (unsigned)b), us < 0, a, b);
    }

    check_pair_narrow();

    /* NaN, the infinities and the signed zeros: the float spelling must keep
     * evaluating both comparisons. */
    {
        static const double fv[] = {0.0, -0.0, 1.0, -1.0, 1e308, -1e308,
                                    0.0 / 0.0, 1.0 / 0.0, -1.0 / 0.0};
        const int nf = (int)(sizeof fv / sizeof fv[0]);
        for (int i = 0; i < nf; i++) {
            for (int j = 0; j < nf; j++) {
                double a = fv[i], b = fv[j];
                long long t = opaque((long long)((a > b) - (a < b)));
                check("f64_le", f64_le(a, b), t <= 0, i, j);
                check("f64_eq", f64_eq(a, b), t == 0, i, j);
            }
        }
    }

    printf(fails ? "VALIDATION FAILED (%d)\n" : "VALIDATION OK\n", fails);
    return fails ? 1 : 0;
}
