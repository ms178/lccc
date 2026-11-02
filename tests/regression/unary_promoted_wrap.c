/* Unary `-`, `~` and `+` wrap at the operand's PROMOTED type (C11
 * 6.5.3.3p1/p4), which on LP64 is frequently narrower than the operand
 * expression's IR carrier.
 *
 * The lowering used to compute `-x` and `~x` at the carrier width: an `int`
 * literal is carried as I64, so `12 * 0xFFFFu` -- a plain `unsigned int` --
 * arrives as a 64-bit operand, and the negation ran as `negq` instead of
 * `negl`. The low 32 bits were right; the rest was the 64-bit two's complement
 * of the 32-bit result, and that sign leaked into every widening context
 * (`(long long)`, `(unsigned long long)`, `(double)`, an assignment). The
 * expression fuzzer found it as
 *
 *     printf("%lld\n", (long long)(((7 >> 5) != (0xFFFFu && 255)),
 *                                  (-((12 * 0xFFFFu)))));
 *
 * which printed -786420 where gcc-16 and clang-23 print 4294180876.
 *
 * Every check below is a value where the 32-bit and the 64-bit computation
 * disagree, so a reintroduced carrier-width negation or complement cannot hide
 * behind a truncating destination.  `sink` is volatile: the operands must come
 * from memory, so nothing is constant-folded away and the runtime path is the
 * one that runs.  The comma forms pin the path that the fuzzer took, where the
 * enclosing comma keeps sema from folding the operand in the first place.
 *
 * The expected values are printed, not asserted: run_regression.py compares
 * stdout against the oracle (gcc-16), which is what "correct" means here.
 */

#include <stdio.h>

static volatile unsigned u_sink = 0xFFFFFFF0u;
static volatile int i_sink = -7;

/* Widening contexts, all fed by the same 32-bit result. */
static void report_unsigned(const char *tag, unsigned v) {
    long long as_ll = (long long)-(v + 1);
    unsigned long long as_ull = (unsigned long long)-(v + 1);
    long long not_ll = (long long)~(v + 1);
    unsigned long long not_ull = (unsigned long long)~(v + 1);
    double as_d = (double)-(v + 1);
    long long stored = -(v * 1);
    printf("%s v=%u neg_ll=%lld neg_ull=%llu not_ll=%lld not_ull=%llu neg_d=%.1f mul=%lld\n",
           tag, v, as_ll, as_ull, not_ll, not_ull, as_d, stored);
}

int main(void) {
    unsigned u = u_sink;
    int i = i_sink;

    report_unsigned("sink", u);
    /* The literal operand is what widens the carrier: 1 is an int (I64
     * carrier), `u` an unsigned int, and the usual arithmetic conversions
     * make `u + 1` unsigned int. */
    report_unsigned("lit", 0xFFFFFFF0u);
    report_unsigned("one", 1u);
    report_unsigned("zero", 0u);
    report_unsigned("alias", u == 0u ? 3u : 0xFFFFFFF0u);

    /* Literal-first spelling and other operators over the same layout. */
    printf("lit_first %lld %llu\n",
           (long long)-(1 + u), (unsigned long long)-(1 + u));
    printf("or_zero %lld %llu\n",
           (long long)-(u | 0), (unsigned long long)-(u | 0));
    printf("sub_zero %lld %llu\n",
           (long long)-(u - 0), (unsigned long long)-(u - 0));
    printf("shift %lld %llu\n",
           (long long)-(u >> 0), (unsigned long long)-(u >> 0));
    printf("cast_ui %lld %llu\n",
           (long long)(unsigned int)-(u + 1), (unsigned long long)(unsigned int)~(u + 1));

    /* Signed operands: `-(i + 1)` and `-(i * 1)` are `int`, so the widening
     * conversion sign-extends a 32-bit result. */
    printf("signed %lld %lld %lld\n",
           (long long)-(i + 1), (long long)-(i * 1), (long long)~(i + 1));

    /* Conditional with a literal arm: the conditional's type is `int`, so the
     * negation wraps at 32 bits even though the storage type is 64-bit. */
    printf("cond %lld %lld\n",
           (long long)-(i ? i : 1), (long long)~(i ? i : 1));

    /* Comma: the enclosing comma keeps sema from folding the operand, which is
     * the exact shape the fuzzer produced. */
    {
        int zero = (int)u_sink - (int)u_sink;
        printf("comma %lld %lld %llu\n",
               (long long)(zero, -(12 * 0xFFFFu)),
               (long long)(zero, ~(12 * 0xFFFFu)),
               (unsigned long long)(zero, -(12 * 0xFFFFu)));
        printf("comma_var %lld %lld\n",
               (long long)(zero, -(u + 1)),
               (long long)(zero, ~(u + 1)));
    }

    /* Assignment and array-store paths (the value crosses a slot). */
    {
        long long slots[4];
        unsigned long long uslots[4];
        slots[0] = -((long long)u + 1);
        slots[1] = (long long)-(u + 1);
        uslots[0] = (unsigned long long)-(u + 1);
        slots[2] = (long long)~(u + 1);
        uslots[1] = (unsigned long long)~(u + 1);
        slots[3] = (long long)-(u * 1);
        uslots[2] = (unsigned long long)-(u | 0);
        uslots[3] = (unsigned long long)~(u - 0);
        printf("slots %lld %lld %lld %lld %llu %llu %llu %llu\n",
               slots[0], slots[1], slots[2], slots[3],
               uslots[0], uslots[1], uslots[2], uslots[3]);
    }

    return 0;
}
