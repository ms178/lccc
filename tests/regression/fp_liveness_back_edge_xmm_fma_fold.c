#include <stdio.h>
/* Peephole FpLiveness regression (lc2): commutative sibling of the VEX
 * memory fold plus the FMA memory-src2 fold (`h`: the loop-top consumer is
 * an FMA of prev).  Expected "102 1046" at -O2 -mno-avx and -O1. */
__attribute__((noinline)) double f(const double *a, int n, double acc, double acc2) {
    double prev = 0.0;
    for (int i = 0; i < n; i++) {
        acc += prev * 3.0;
        prev = a[i];
        acc2 += prev;
    }
    return acc + acc2 * 0.5;
}
__attribute__((noinline)) double h(const double *a, int n, double acc, double acc2) {
    double prev = 0.0;
    for (int i = 0; i < n; i++) {
        acc = acc * prev + 1.0;
        prev = a[i];
        acc2 = acc2 * prev;
    }
    return acc + acc2;
}
int main(void) {
    double a[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    printf("%g %g\n", f(a, 8, 0.0, 0.0), h(a, 6, 1.0, 1.0));
    return 0;
}
