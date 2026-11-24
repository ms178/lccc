/* Loop rotation when the exit block already has phis.
 *
 * An inlined function that ends in a loop leaves no block between its loop
 * and what follows the call: the loop's header exits straight to the next
 * loop's header.  Rotation takes the loops one at a time and looks for them
 * again after each, so the second loop is rotated first (the first one's
 * exit block has two predecessors until then).  The second loop's header is
 * then a guard with one predecessor and one-incoming phis:
 *
 *     first.header:  i = phi ...          -> first.body, second.guard
 *     second.guard:  s = phi [0, first.header]
 *                    w = phi [w1, first.header]      (w1: a phi of first)
 *
 * Rotating the first loop makes its latch a second predecessor of
 * second.guard.  Those phis got no incoming for that edge, and the one they
 * had was rewritten to the merge phi rotation puts beside them:
 *
 *     SSA PHI-ARITY VIOLATION ... block 6 phi incoming pred labels {13}
 *         violates the reachable-pred contract (missing [14])
 *     SSA PHI-DOMINANCE VIOLATION ... phi v66 incoming v75 from pred block 1
 *         is not dominated by the defining block 3
 *
 * (CCC_LOOP_ROTATE=1 CCC_VALIDATE_SSA=1: seven tests of this directory and
 * of tests/benchmark/programs did not compile.  Without the validator they
 * printed the right numbers: the CFG cleanup that follows turns a
 * one-incoming phi into a copy of what it names, and the name was the
 * merge.)
 *
 * Each function below is one of the values such a phi takes from the first
 * loop: a constant, a value from in front of both loops, the first loop's
 * accumulator, its counter, and two values it swaps.  The .env file turns
 * rotation and the validator on; run_regression_suite diffs against GCC.
 */
#include <stdio.h>

static inline void smear(unsigned char *p, unsigned long k, unsigned long n) {
    for (unsigned long i = k; i < n; i++) p[i + 1] = p[i];
}

static inline void tally(const unsigned char *p, unsigned long k, unsigned long n,
                         unsigned long *out) {
    for (unsigned long i = k; i < n; i++) *out = *out * 3 + p[i];
}

static inline void advance(const unsigned char *p, unsigned long n, unsigned long *pos) {
    while (*pos < n) *pos += 1 + (p[*pos] & 1);
}

static inline void swap_walk(unsigned long n, unsigned long *x, unsigned long *y) {
    for (unsigned long j = 0; j < n; j++) {
        unsigned long t = *x;
        *x = *y + j;
        *y = t;
    }
}

/* The second loop starts from constants. */
__attribute__((noinline)) unsigned long after_const(unsigned char *p, unsigned long k,
                                                    unsigned long n) {
    smear(p, k, n);
    unsigned long s = 0;
    for (unsigned long i = 0; i < n + 2; i++) s = s * 31 + p[i];
    return s;
}

/* ... from a value computed in front of both loops. */
__attribute__((noinline)) unsigned long after_seed(unsigned char *p, unsigned long k,
                                                   unsigned long n, unsigned long seed) {
    unsigned long s = seed * 3 + 1;
    smear(p, k, n);
    for (unsigned long j = 0; j < n + 2; j++) s = s * 31 + p[j];
    return s;
}

/* ... from the first loop's accumulator. */
__attribute__((noinline)) unsigned long after_tally(const unsigned char *p, unsigned long k,
                                                    unsigned long n, unsigned long m) {
    unsigned long w = k;
    tally(p, k, n, &w);
    for (unsigned long i = 0; i < m; i++) w = w * 31 + p[i];
    return w;
}

/* ... from the first loop's counter, which is read behind both as well. */
__attribute__((noinline)) unsigned long after_advance(const unsigned char *p, unsigned long k,
                                                      unsigned long n, unsigned long m) {
    unsigned long pos = k;
    advance(p, n, &pos);
    unsigned long s = 5;
    for (; pos < m; pos++) s = s * 31 + p[pos];
    return s + pos;
}

/* ... from two values the first loop swaps: what leaves is the other one's
 * value of the last iteration. */
__attribute__((noinline)) unsigned long after_swap(unsigned long n, unsigned long m) {
    unsigned long x = 1, y = 2;
    swap_walk(n, &x, &y);
    for (unsigned long j = 0; j < m; j++) {
        x = x * 5 + y;
        y ^= x >> 3;
    }
    return x * 31 + y;
}

/* Three in a row: the middle loop is the exit of the first and has the
 * third as its own. */
__attribute__((noinline)) unsigned long three_in_a_row(unsigned char *p, unsigned long k,
                                                       unsigned long n, unsigned long m) {
    unsigned long w = n;
    smear(p, k, n);
    tally(p, k, m, &w);
    for (unsigned long i = 0; i < n; i++) w = w * 7 + p[i];
    return w;
}

int main(void) {
    unsigned char b[96];
    unsigned long sum = 0;
    for (unsigned long n = 0; n < 44; n++) {
        for (unsigned i = 0; i < 96; i++) b[i] = (unsigned char)(i * 37 + n * 5 + 11);
        unsigned long r1 = after_const(b, n / 2, n);
        unsigned long r2 = after_seed(b, n / 5, n, n * 1000003);
        unsigned long r3 = after_tally(b, n / 3, n, n + 2);
        unsigned long r4 = after_advance(b, n / 4, n, n + 9);
        unsigned long r5 = after_swap(n, n / 2 + 1);
        unsigned long r6 = three_in_a_row(b, n / 6, n, n + 3);
        if (n < 6 || n % 9 == 0)
            printf("%lu: %lu %lu %lu %lu %lu %lu\n", n, r1, r2, r3, r4, r5, r6);
        sum = sum * 1099511628211ul + (r1 ^ r2 ^ r3 ^ r4 ^ r5 ^ r6);
    }
    /* No trip, one trip and a start behind the bound. */
    for (unsigned i = 0; i < 96; i++) b[i] = (unsigned char)(200 - i);
    printf("edge: %lu %lu %lu %lu %lu\n", after_const(b, 5, 5), after_tally(b, 9, 3, 0),
           after_tally(b, 3, 4, 1), after_advance(b, 50, 10, 50), after_swap(0, 0));
    printf("sum %lu\n", sum);
    return 0;
}
