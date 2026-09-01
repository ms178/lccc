/* The loop counter after a VECTORIZED loop.
 *
 * Vectorizing redefines what the counter counts. Under the byte-offset scheme
 * it steps `elem_size * vec_width` BYTES per iteration; under the
 * element-index scheme the trip count is divided so it numbers VECTOR
 * iterations. Inside the loop only addresses read it, so neither is visible
 * there -- but a use AFTER the loop reads a number that is no longer the
 * element index. `for (; n < max; n++) acc += v[n]; return acc + (n >> 2);`
 * returned 528 (byte scheme, n == 128) or 497 (element scheme, n == 4) where
 * GCC 16.2, Clang 23.1, ICC and ICX all return 504 (n == 32).
 *
 * The repair costs nothing: the transform already builds a scalar remainder
 * loop whose induction variable counts ELEMENTS and runs to the original trip
 * bound, so its exit value IS the counter's correct final value. Escaping uses
 * are rewired to it -- no arithmetic is synthesized and the loop stays
 * vectorized.
 *
 * The boundaries below are where a counter-reconstruction scheme breaks, and
 * each is checked across a sweep of trip counts rather than one lucky value:
 *
 *   - trip count an EXACT MULTIPLE of the vector width, so the remainder loop
 *     runs ZERO times and its phi must still carry the preheader's start value;
 *   - trip count BELOW the vector width, so the VECTOR loop runs zero times;
 *   - trip count ZERO, where the counter must remain 0;
 *   - element sizes 4 and 8, which scale the byte counter differently;
 *   - the counter read several times, and read as a value rather than just
 *     returned;
 *   - an early-exit loop, where the counter's final value is not the bound.
 *
 * Every result is compared against GCC by the regression harness, so a wrong
 * counter shows up as an oracle mismatch rather than needing a hard-coded
 * expectation per case.
 */
#include <stdio.h>

/* The original defect: counter feeds arithmetic after the loop. */
static int reduce_and_shift(const int *v, int max) {
    int acc = 0;
    int n = 0;
    for (; n < max; n++) {
        acc += v[n];
    }
    return acc + (n >> 2);
}

/* Counter returned unmodified. */
static int reduce_and_count(const int *v, int max) {
    int acc = 0;
    int n = 0;
    for (; n < max; n++) {
        acc += v[n];
    }
    return acc * 1000 + n;
}

/* Counter read more than once, in different expressions. */
static int reduce_multi_use(const int *v, int max) {
    int acc = 0;
    int n = 0;
    for (; n < max; n++) {
        acc += v[n];
    }
    return acc + (n >> 1) + (n & 7) + (n * 3);
}

/* 8-byte elements: the byte counter scales by 8, not 4. */
static long long reduce_i64(const long long *v, int max) {
    long long acc = 0;
    int n = 0;
    for (; n < max; n++) {
        acc += v[n];
    }
    return acc + n;
}

/* Two arrays (dot-product shape) with an escaping counter. */
static int dot_and_count(const int *a, const int *b, int max) {
    int acc = 0;
    int n = 0;
    for (; n < max; n++) {
        acc += a[n] * b[n];
    }
    return acc + n;
}

/* Unsigned counter. */
static unsigned reduce_unsigned(const int *v, unsigned max) {
    int acc = 0;
    unsigned n = 0;
    for (; n < max; n++) {
        acc += v[n];
    }
    return (unsigned) acc + n;
}

/* The counter compared, not just consumed arithmetically. */
static int reduce_and_compare(const int *v, int max) {
    int acc = 0;
    int n = 0;
    for (; n < max; n++) {
        acc += v[n];
    }
    return (n == max) ? acc : -1;
}

/* Early exit: the counter's final value is NOT the loop bound, so no
 * `trips * vec_width` formula can produce it -- only the real counter can. */
static int find_first_negative(const int *v, int max) {
    int n = 0;
    while (n < max) {
        if (v[n] < 0) {
            break;
        }
        n++;
    }
    return n;
}

int main(void) {
    enum { N = 80 };
    int v[N];
    int b[N];
    long long w[N];
    for (int i = 0; i < N; i++) {
        v[i] = i;
        b[i] = (i % 5) - 2;
        w[i] = (long long) i * 3;
    }

    /* Sweep across every residue class modulo the widest vector (8 for i32
     * under AVX2, 4 for i64), plus zero and sub-width trip counts. One lucky
     * multiple would hide exactly the bug this file exists for. */
    long long h1 = 0, h2 = 0, h3 = 0, h4 = 0, h5 = 0, h6 = 0, h7 = 0;
    for (int max = 0; max <= 34; max++) {
        h1 = h1 * 31 + reduce_and_shift(v, max);
        h2 = h2 * 31 + reduce_and_count(v, max);
        h3 = h3 * 31 + reduce_multi_use(v, max);
        h4 = h4 * 31 + reduce_i64(w, max);
        h5 = h5 * 31 + dot_and_count(v, b, max);
        h6 = h6 * 31 + (long long) reduce_unsigned(v, (unsigned) max);
        h7 = h7 * 31 + reduce_and_compare(v, max);
    }

    /* Early exit at a range of positions, including before, at and after the
     * first vector boundary. */
    long long h8 = 0;
    for (int pos = 0; pos < 24; pos++) {
        int tmp[N];
        for (int i = 0; i < N; i++) {
            tmp[i] = 1;
        }
        tmp[pos] = -1;
        h8 = h8 * 31 + find_first_negative(tmp, 40);
    }

    printf("%lld %lld %lld %lld %lld %lld %lld %lld\n", h1, h2, h3, h4, h5, h6, h7, h8);
    return 0;
}
