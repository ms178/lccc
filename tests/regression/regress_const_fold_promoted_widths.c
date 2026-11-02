/* Unary minus and bitwise NOT on unsigned constants must reduce modulo
 * 2^width of the operand's PROMOTED type (C11 6.5.3.3p1/p4 with
 * 6.3.1.3p2), in both constant folders.
 *
 * The failure this guards is a carrier bug, not an arithmetic one: the
 * evaluators store `unsigned int` in a 64-bit carrier so the full range
 * survives, and a carrier-wide negation then computes mod 2^64. One site
 * (lowering `~`) even guarded on a different type query than the other
 * (sema `~`), so the same expression could fold differently depending on
 * which evaluator reached it first. The rules now live in
 * common::const_eval, and this test compares every width's folded value
 * against the value computed at run time from a volatile -- the two must
 * agree, and both must agree with GCC.
 *
 * Widths covered: unsigned char/short (which promote to int, so the
 * result is an int and must NOT be reduced to the operand's width),
 * unsigned int (the carrier case), unsigned long/long long, and
 * unsigned __int128.
 *
 * Expected output (identical under GCC 16.2 at -O0/-O2/-Os, and -m32):
 *   450364216 450364216 4294967295 450364215
 *   18446744073709551574 18446744073709551574 18446744073709551615 18446744073709551573
 *   -200 -200 -65535 -65535
 *   18446744073709551574 18446744073709551574
 *   1 1 1 1 1
 *   1
 *   0 0
 *   18446744073709551615 18446744073709551615 18446744073709551615 18446744073709551615 18446744073709551615 18446744073709551615
 *   4294967295 4294967295 4294967296 4294967295
 *   4294967295 4294967295 4294967296 4294967295
 *   4294967295 4294967295 4294967296 4294967295
 *   18446744073709551613 18446744073709551613
 *
 * Read the second line as the width proof: -(42ULL) is 2^64-42, and
 * ~(42ULL) is 2^64-43. A fold that had reduced the 64-bit case to 32
 * bits would print 4294967254 and 4294967253 here instead.
 *
 * Lines 9-11 are the carrier witnesses. They are the part of this test
 * that fails for the bug it was written for: with the 2^32 reduction
 * removed from `negate_const_promoted`/`bitnot_const_promoted`, lines 9
 * and 10 print 18446744073709551615 18446744073709551615 0
 * 18446744073709551615 (verified by mutation), while the run-time line
 * stays correct. The exit status is 13 in that case (the `k_widen_neg_one`
 * check), measured by mutation.
 *
 * Line 12 is the mirror-image witness: a SIGNED 32-bit operand must not be
 * reduced, because the later widening sign-extends it. With the reduction
 * applied unconditionally it prints 4294967293 4294967293 and the program
 * exits 17 (also measured by mutation).
 */

int printf(const char *, ...);

volatile unsigned int vu = 3844603080U;
volatile unsigned int vu42 = 42U;
volatile unsigned long vul = 42UL;
volatile unsigned char vuc = 200;
volatile unsigned short vus = 65535;
volatile unsigned long long vull = 42ULL;
#ifdef __SIZEOF_INT128__
volatile unsigned __int128 vu128 = 1;
#endif

/* Folded at compile time: static initializers, a case label, an array
 * bound and a ternary condition, i.e. every consumer of the folders. */
static const unsigned int k_neg_u = -(3844603080U);
static const unsigned int k_not_u = ~0U;
static const int k_neg_uc = -(unsigned char)200;
static const int k_neg_us = -(unsigned short)65535;
static const unsigned long long k_neg_ull = -(42ULL);
static const _Bool k_bool_1 = (_Bool)256;
static const _Bool k_bool_2 = (_Bool)65536u;
static const _Bool k_bool_3 = (_Bool)67106816ULL;
static const _Bool k_bool_4 = (_Bool)0;
#ifdef __SIZEOF_INT128__
static const _Bool k_bool_5 = (_Bool)((unsigned __int128)1 << 100);
#endif

enum { k_case_neg = -(3844603080U) ? 7 : 9 };

/* ── Carrier-width witnesses ────────────────────────────────────────────
 * A 32-bit fold is carried in a 64-bit word.  A "%u" printf or an
 * assignment to `unsigned int` cannot see a carrier leak, because both
 * convert modulo 2^32.  Consuming the folded value at 64-bit width can:
 * if the reduction to the promoted type's width were missing, `~0U`
 * would fold to 0xFFFFFFFF_FFFFFFFF and `-(1U)` to the same, so these
 * constants would print 18446744073709551615 instead of 4294967295.
 * They are the sharpest available oracle for the bug this file exists
 * to prevent, and they are the reason it is not a "%u" test.
 */
volatile unsigned int vzero = 0U;
volatile unsigned int vone = 1U;
volatile int vsigned = 3;

static const unsigned long long k_widen_not_u = (unsigned long long)~0U;
static const unsigned long long k_widen_neg_one = (unsigned long long)-(1U);
static const unsigned long long k_widen_plus = ~0U + 1ULL;
static const unsigned long long k_widen_neg_plus = -(1U) + 0ULL;

/* The mirror-image witness: a SIGNED 32-bit operand is carried in the
 * same 64-bit word whenever the surrounding expression is 64-bit, so the
 * carrier width is not evidence of unsignedness.  Masking it to 2^32 here
 * zero-extends a negative value: `(size_t)-(enum{3})` printed 4294967293
 * instead of 18446744073709551613 (a libFSE early-return value).  The
 * run-time twin is widened the same way, so this compares fold against
 * execution rather than against a hand-written constant alone.
 */
enum { k_neg_enum = 3 };
static const unsigned long long k_widen_neg_signed =
    (unsigned long long)-(k_neg_enum);

int main(void) {
    /* unsigned int: the 64-bit carrier must not leak into the result. */
    printf("%u %u %u %u\n", -(3844603080U), (unsigned)-vu, ~0U, (unsigned)~vu);

    /* unsigned long long: full 64-bit modular arithmetic, unmasked. */
    printf("%llu %llu %llu %llu\n",
           (unsigned long long)-(42ULL),
           (unsigned long long)-vull,
           (unsigned long long)~0ULL,
           (unsigned long long)~vull);

    /* Sub-int operands promote to int, so these are SIGNED ints. */
    printf("%d %d %d %d\n",
           -(unsigned char)200, -vuc,
           -(unsigned short)65535, -vus);

    /* unsigned long on LP64 is 64-bit; on ILP32 it is 32-bit. Both are
     * modular in their own width, which is what -vul computes too. */
    printf("%llu %llu\n",
           (unsigned long long)-(unsigned long)42,
           (unsigned long long)-vul);

    /* _Bool conversion is by value: the low byte and the low word are
     * both zero in k_bool_2/k_bool_3, and k_bool_5 has zero low 64 bits. */
    printf("%d %d %d %d %d\n",
           (int)k_bool_1, (int)(_Bool)256,
           (int)k_bool_2, (int)(_Bool)65536u,
           (int)k_bool_3);
#ifdef __SIZEOF_INT128__
    printf("%d\n", (int)k_bool_5);
#endif
    printf("%d %d\n", (int)k_bool_4, (int)(_Bool)0);

    /* unsigned __int128: negation and complement wrap in 128 bits. */
#ifdef __SIZEOF_INT128__
    {
        unsigned __int128 a = -(unsigned __int128)1;
        unsigned __int128 b = ~(unsigned __int128)0;
        unsigned __int128 c = -vu128;
        unsigned long long lo = (unsigned long long)a;
        unsigned long long hi = (unsigned long long)(a >> 64);
        unsigned long long blo = (unsigned long long)b;
        unsigned long long bhi = (unsigned long long)(b >> 64);
        unsigned long long clo = (unsigned long long)c;
        unsigned long long chi = (unsigned long long)(c >> 64);
        printf("%llu %llu %llu %llu %llu %llu\n",
               hi, lo, bhi, blo, chi, clo);
    }
#endif

    /* The carrier witnesses: compile-time fold vs run-time twin at 64-bit
     * width, where a missing 2^32 reduction is visible. */
    printf("%llu %llu %llu %llu\n",
           k_widen_not_u, k_widen_neg_one, k_widen_plus, k_widen_neg_plus);
    printf("%llu %llu %llu %llu\n",
           (unsigned long long)~0U, (unsigned long long)-(1U),
           ~0U + 1ULL, -(1U) + 0ULL);
    printf("%llu %llu %llu %llu\n",
           (unsigned long long)~vzero, (unsigned long long)-vone,
           ~vzero + 1ULL, -vone + 0ULL);
    printf("%llu %llu\n", k_widen_neg_signed,
           (unsigned long long)-(vsigned - vsigned + 3));

    /* Every folded form must equal its run-time twin, and the constant
     * must equal the closed-form value the standard prescribes. */
    if (k_neg_u != (unsigned)-vu || k_neg_u != 450364216u) return 1;
    if (k_not_u != (unsigned)~0U || k_not_u != 4294967295u) return 2;
    if ((unsigned)~vu != 450364215u) return 3; /* 2^32-1 - 3844603080 */
    if (k_neg_ull != (unsigned long long)-vull) return 4;
    if (k_neg_ull != 18446744073709551574ULL) return 5;
    if (k_neg_uc != -vuc || k_neg_uc != -200) return 6;
    if (k_neg_us != -vus || k_neg_us != -65535) return 7;
    if (k_bool_1 != 1 || k_bool_2 != 1 || k_bool_3 != 1 || k_bool_4 != 0) return 8;
#ifdef __SIZEOF_INT128__
    if (k_bool_5 != 1) return 9;
#endif
    if (k_case_neg != 7) return 10;

    /* Carrier witnesses: the fold must agree with the run-time twin AND
     * with the closed form 2^32-1 (not 2^64-1, which is what a leak
     * produces). */
    if (k_widen_not_u != (unsigned long long)~vzero) return 11;
    if (k_widen_not_u != 4294967295ULL) return 12;
    if (k_widen_neg_one != (unsigned long long)-vone) return 13;
    if (k_widen_neg_one != 4294967295ULL) return 14;
    if (k_widen_plus != ~vzero + 1ULL || k_widen_plus != 4294967296ULL) return 15;
    if (k_widen_neg_plus != -vone + 0ULL || k_widen_neg_plus != 4294967295ULL) return 16;
    /* Signed operand carried in I64: 2^64-3, not 2^32-3. */
    if (k_widen_neg_signed != 18446744073709551613ULL) return 17;
    if (k_widen_neg_signed !=
        (unsigned long long)-(vsigned - vsigned + 3)) return 18;
    return 0;
}
