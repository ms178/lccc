/* Unary minus on an unsigned int constant must reduce modulo 2^32
 * (C11 6.5.3.3p1: the result has the promoted type of the operand;
 * 6.3.1.3p2: unsigned arithmetic is modular).  Both constant folders
 * (sema and lowering) carried a 32-bit unsigned operand as I64 and
 * negated the carrier, computing -x mod 2^64: -(3844603080U) folded to
 * 18446744069864948536 instead of 450364216, and -(42U) to
 * 18446744073709551574 instead of 4294967254.  A folded min() then took
 * the wrong arm.  Runtime negation was always correct; only the fold
 * was wrong.  BitNot already had the width treatment; regression
 * regress_logical_and_const_bitnot_short_circuit covers that sibling.
 *
 * Found by scripts/yarpgen_mutpair.py (intel/yarpgen seed 101,
 * original program, -O0): the single diverging global was written by
 *   var_419 = max(var_419, (long long) min(min(a, (ull)b),
 *                                           (ull)(-(3844603080U))))
 * Expected program output (GCC-verified, all -O levels):
 * "450364216 4294967254 999983 999983" -- the last two are the folded
 * and runtime min/max results, which must agree (both lose to the
 * smaller first min() arm 999983; before the fix the folded form
 * returned 5632918028137529527 because the folded negation was huge). */

int printf(const char *, ...);

#define MAX(a, b) ({ __typeof__(a) _a = (a); __typeof__(b) _b = (b); _a > _b ? _a : _b; })
#define MIN(a, b) ({ __typeof__(a) _a = (a); __typeof__(b) _b = (b); _a < _b ? _a : _b; })

static const unsigned long long static_neg = (unsigned long long)(-(3844603080U));
volatile unsigned int vu = 3844603080U;

int main(void) {
    unsigned long long direct = (unsigned long long)(-(3844603080U));
    unsigned long long small = (unsigned long long)(-(42U));

    /* The seed's shape: the folded negation must win the min(), exactly
     * like the runtime (volatile) form next to it. */
    long long folded = MAX(-855359829999662162LL,
        (long long) MIN(MIN(5632918028137529527LL, (unsigned long long)999983ULL),
                        (unsigned long long)(-(3844603080U))));
    long long runtime = MAX(-855359829999662162LL,
        (long long) MIN(MIN(5632918028137529527LL, (unsigned long long)999983ULL),
                        (unsigned long long)(-vu)));

    /* Folded and runtime must agree; sum with the two direct folds and
     * the static initializer pins every folding site at once. */
    printf("%llu %llu %lld %lld\n", direct, small, folded,
           runtime == folded && static_neg == direct ? folded : -1LL);
    return 0;
}
