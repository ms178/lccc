/* Guarded do-while loops with VALUE exit bounds: the shapes
 * fold_loop_value_bound must fold, the shapes it must refuse, and the
 * boundary values of `n` where an unsound fold diverges (underflow of
 * `n - C` near the type minimum, zero-trip guards).  The runtime contract
 * is bit-exact stdout parity with the GCC oracle. */
#include <stdio.h>
#include <limits.h>

/* FOLD: signed, affine chained onto the post-increment. */
__attribute__((noinline))
long sum_dw(long n) {
    long s = 0, i = 0;
    if (i + 4 < n) { do { s += i * 3 + 1; i++; } while (i + 4 < n); }
    return s;
}

/* FOLD: unsigned domain. */
__attribute__((noinline))
unsigned long scan_dw_u(unsigned long n) {
    unsigned long acc = 1, i = 0;
    if (i + 2 < n) { do { acc = acc * 31 + i; i++; } while (i + 2 < n); }
    return acc;
}

/* REFUSE: non-strict latch compare (<=). */
__attribute__((noinline))
long le_dw(long n) {
    long s = 0, i = 0;
    if (i + 4 <= n) { do { s += i; i++; } while (i + 4 <= n); }
    return s;
}

/* REFUSE: step 2. */
__attribute__((noinline))
long step2_dw(long n) {
    long s = 0, i = 0;
    if (i + 4 < n) { do { s += i; i += 2; } while (i + 4 < n); }
    return s;
}

/* FOLD (inverted latch): a plain for loop; loop inversion duplicates the
 * top test into the latch and the fold rewrites the duplicate against the
 * hoisted bound. */
__attribute__((noinline))
long toptest(long n) {
    long s = 0;
    for (long i = 0; i + 4 < n; i++) s += i ^ (i >> 1);
    return s;
}

/* REFUSE: the -fwrapv re-entry trap.  `i` survives across outer iterations,
 * so the guard's own `i + 4` may wrap on re-entry; the seed is not constant
 * per entry and the fold must decline. */
__attribute__((noinline))
long reentry(long seed, long n, int outer) {
    long i = seed, s = 0;
    for (int j = 0; j < outer; j++) {
        for (; i + 4 < n; i++) { s += 1; if (s > 8) break; }
        s += 100;
        n -= 1;
    }
    return s ^ i;
}

int main(void) {
    const long edges[] = {LONG_MIN, LONG_MIN + 3, LONG_MIN + 4, LONG_MIN + 5,
                          -7, -1, 0, 1, 3, 4, 5, 6, 7, 20, 50};
    const unsigned long uedges[] = {0, 1, 2, 3, 4, 5, 20, 50};
    for (unsigned k = 0; k < sizeof edges / sizeof *edges; k++) {
        long n = edges[k];
        printf("n=%ld sum=%ld le=%ld s2=%ld tt=%ld re=%ld\n",
               n, sum_dw(n), le_dw(n), step2_dw(n), toptest(n),
               reentry(n, 12, 3) + reentry(LONG_MAX - 2, n, 2));
    }
    for (unsigned k = 0; k < sizeof uedges / sizeof *uedges; k++) {
        unsigned long n = uedges[k];
        printf("un=%lu scan=%lu\n", n, scan_dw_u(n));
    }
    puts("affine-value-bound: all shapes exact");
    return 0;
}
