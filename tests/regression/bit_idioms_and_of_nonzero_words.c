/* bit_idioms: the truth of a bitwise AND.
 *
 * The fact engine's `truth` answered "true" for an AND whose operands it
 * knew to be non-zero.  That is right for two Booleans (both are 1) and for
 * nothing else: `(x | 1) & (y | 2)` is 0 for `x == y == 0`.  A select on
 * such a word was taken to choose its first arm, and behind any guard (the
 * comparison fold wanted a fact from a dominating edge) a comparison of the
 * select folded:
 *
 *     unsigned t = ((x | 1) & (y | 2)) ? 4 : 8;
 *     if (x < 100) return t == 4;          // -O2, -O3, -Os: always 1
 *
 * guarded(0, 0) returned 1 instead of 0.  Two words are known to share a
 * bit only through their bits (`(x | 1) & (y | 3)`), and that is what the
 * engine asks now.  Every function below is one way to reach the question;
 * `shared_*` are the ones whose AND really is never zero.  The grid in
 * `main` holds the pairs on which the words share no bit;
 * run_regression_suite diffs against GCC.
 */
#include <stdio.h>

/* The shape that was found. */
__attribute__((noinline)) int guarded(unsigned x, unsigned y) {
    unsigned t = ((x | 1) & (y | 2)) ? 4 : 8;
    if (x < 100) return t == 4;
    return -1;
}

/* Written as a statement, which if-conversion turns into the same select. */
__attribute__((noinline)) int guarded_statement(unsigned x, unsigned y) {
    unsigned t = 8;
    if ((x | 1) & (y | 2)) t = 4;
    if (y < 100) return t == 4;
    return -1;
}

/* The select's value is used, not compared. */
__attribute__((noinline)) unsigned chosen(unsigned x, unsigned y) {
    unsigned t = ((x | 4) & (y | 8)) ? 48u : 80u;
    if ((x & 0x100) == 0) return t + (t >> 4);
    return 1;
}

/* Three words: any two may share a bit while all three do not. */
__attribute__((noinline)) int three(unsigned x, unsigned y, unsigned z) {
    unsigned t = ((x | 3) & (y | 6) & (z | 12)) ? 4 : 8;
    if (z < 1000) return t == 4;
    return -1;
}

/* 64 bits wide, and the words come from shifts. */
__attribute__((noinline)) int wide(unsigned long x, unsigned long y) {
    unsigned long t = ((x << 1 | 1) & (y << 2 | 2)) ? 4 : 8;
    if (x < 1000) return t == 4;
    return -1;
}

/* The AND is the condition of a count: `ctz` of the arm that was chosen. */
__attribute__((noinline)) int counted(unsigned x, unsigned y) {
    unsigned a = x | 1, b = y | 2;
    unsigned t = (a & b) ? 4u : 8u;
    return __builtin_ctz(t);
}

/* No guard at all. */
__attribute__((noinline)) int bare(unsigned x, unsigned y) {
    return (((x | 1) & (y | 2)) ? 4 : 8) == 4;
}

/* These share a bit whatever the arguments are. */
__attribute__((noinline)) int shared_bit(unsigned x, unsigned y) {
    unsigned t = ((x | 1) & (y | 3)) ? 4 : 8;
    if (x < 100) return t == 4;
    return -1;
}

__attribute__((noinline)) int shared_booleans(unsigned x, unsigned y) {
    unsigned a = x | 1, b = y | 2;
    unsigned t = ((a != 0) & (b != 0)) ? 4 : 8;
    if (x < 100) return t == 4;
    return -1;
}

int main(void) {
    static const unsigned grid[] = {0, 1, 2, 3, 4, 8, 12, 16, 99, 100, 255, 256, 0x80000000u, 0xfffffffeu};
    enum { N = sizeof grid / sizeof grid[0] };
    unsigned long sum = 0;
    for (unsigned i = 0; i < N; i++) {
        for (unsigned j = 0; j < N; j++) {
            unsigned x = grid[i], y = grid[j];
            int r[] = {
                guarded(x, y), guarded_statement(x, y), (int)chosen(x, y), three(x, y, grid[(i + j) % N]),
                wide(x, y), counted(x, y), bare(x, y), shared_bit(x, y), shared_booleans(x, y),
            };
            for (unsigned k = 0; k < sizeof r / sizeof r[0]; k++)
                sum = sum * 31 + (unsigned)r[k];
            if (i < 3 && j < 3)
                printf("%u %u: %d %d %d %d %d %d %d %d %d\n", x, y, r[0], r[1], r[2], r[3], r[4],
                       r[5], r[6], r[7], r[8]);
        }
    }
    printf("sum %lu\n", sum);
    return 0;
}
