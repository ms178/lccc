/*
 * lea_chain_index_compose.c -- regression test for a peephole that built an
 * x86-64 addressing mode with TWO index registers.
 *
 * `fold_lea_into_load` splices a `leaq`'s address into a later instruction
 * that reads it.  For the indexed consumer form `leaq disp(%T, %idx, %s),
 * %dst` it substituted the producer's operand TEXT into the base slot.  That
 * is sound only when the producer address occupies the base slot itself
 * (`disp(%base)`).  When the producer address carries its own index register
 * -- `leaq 0(,%r11,4), %r8`, the shape strength-reduced division emits for
 * `q*4` -- the two index fields landed side by side:
 *
 *     leaq 0(,%r11,4), %r8                leaq 0(,%r11,4, %r10, 1), %r9
 *     leaq (%r8, %r10, 1), %r9       ==>                       INVALID
 *
 * x86-64 has no such encoding: a SIB byte carries one base (always scale 1)
 * and one index (scale 1/2/4/8).  GAS rejects the line outright ("expecting
 * ')' after scale factor"), but lccc's integrated assembler silently
 * TRUNCATED it to `leaq 0(,%r11,4), %r9`, dropping `+ %r10`.  The miscompile
 * was invisible for that reason: the program assembled, ran and printed
 * plausible numbers while every `q*d + r == n` check really compared `q*d`
 * against `n`.  It surfaced as 341 failures in div_mod_by_const_exhaustive at
 * -O1, only for power-of-two divisors -- only those make `q*d` a scale the
 * folder can absorb.
 *
 * The correct composition keeps both addends and stays at ONE instruction:
 *
 *     leaq (%r10, %r11, 4), %r9
 *
 * exactly what GCC and Clang emit for the same source.
 *
 * Shape notes, each verified by minimization rather than assumed:
 *
 *  - The divisor must reach the division as a PARAMETER that constant-folds
 *    through inlining.  Writing the division directly inside the noinline
 *    function does not reproduce: the fold needs the two-operand lea chain
 *    that inlining leaves behind.
 *  - The printf inside the failure branch is load-bearing.  It is not
 *    decoration: the variadic call is what raises register pressure enough
 *    for the allocation to produce the `leaq chain` the fold then mangles.
 *    Removing it makes the miscompile vanish rather than merely go
 *    unreported, so the checks below stay in this form deliberately.
 *  - The failure branch must stay cold.  The loop bounds below are chosen so
 *    the branch is never taken on a correct compiler, which keeps the test's
 *    output silent on success.
 *
 * -O1 is where the fold fires; the test is compiled at -O1 by its .flags.
 */

#include <stdio.h>

static unsigned long long bad;

static void check_u64(unsigned long long n, unsigned long long d)
{
    unsigned long long q = n / d;
    unsigned long long r = n % d;

    /* `r >= d` or a broken identity means the quotient is off by one, which
     * is how a wrong magic number or shift shows up. */
    if (r >= d || q * d + r != n) {
        bad++;
        printf("FAIL u64 n=%llu d=%llu q=%llu r=%llu\n", n, d, q, r);
    }
}

static void check_s64(long long n, long long d)
{
    long long q = n / d;
    long long r = n % d;
    long long ad = d < 0 ? -d : d;
    long long ar = r < 0 ? -r : r;

    /* C99 truncates toward zero: |r| < |d| and sign(r) == sign(n). */
    if (ar >= ad || (r != 0 && ((r < 0) != (n < 0))) || q * d + r != n) {
        bad++;
        printf("FAIL s64 n=%lld d=%lld q=%lld r=%lld\n", n, d, q, r);
    }
}

/* One noinline wrapper per divisor, so the divisor stays a literal.  Scales
 * 2/4/8 exercise the fold; 3/5/7 are controls whose product is not a scale,
 * so the fold never fires -- if one of those ever fails, the defect is not
 * the one documented here. */
#define W(NAME, D)                                                      \
    __attribute__((noinline)) static void NAME(unsigned long long n)    \
    {                                                                   \
        check_u64(n, (D));                                              \
    }

W(w2, 2ULL)
W(w3, 3ULL)
W(w4, 4ULL)
W(w5, 5ULL)
W(w7, 7ULL)
W(w8, 8ULL)
W(w16, 16ULL)

#define S(NAME, D)                                                      \
    __attribute__((noinline)) static void NAME(long long n)             \
    {                                                                   \
        check_s64(n, (D));                                              \
    }

S(s3, 3LL)
S(s4, 4LL)
S(s8, 8LL)

int main(void)
{
    unsigned long long n;
    long long m;

    for (n = 0; n < 300; n++) {
        w2(n);
        w3(n);
        w4(n);
        w5(n);
        w7(n);
        w8(n);
        w16(n);
    }
    for (m = -300; m < 300; m++) {
        s3(m);
        s4(m);
        s8(m);
    }

    /* Magnitudes where a dropped addend is least likely to be masked. */
    w4(~0ULL);
    w8(~0ULL);
    w2(0x8000000000000000ULL);
    w16(0x8000000000000001ULL);

    return bad ? 1 : 0;
}
