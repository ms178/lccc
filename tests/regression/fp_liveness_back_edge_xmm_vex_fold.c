#include <stdio.h>
/* Peephole FpLiveness regression (lc): fold_fp_register_loads /
 * fold_scalar_fp_memory_into_vex_op fold `movsd MEM, %xmmD; vOP %xmmD, …`
 * into `vOP MEM, …` when %xmmD is dead.  `prev` is loop-carried: its only
 * textual use AFTER the load is the adjacent consumer, but the loop top of
 * the NEXT iteration reads it through the back edge.  Expected 66. */
__attribute__((noinline)) double f(const double *a, int n, double acc, double acc2) {
    double prev = 0.0;
    for (int i = 0; i < n; i++) {
        acc += prev * 3.0;
        prev = a[i];
        acc2 -= prev;
    }
    return acc + acc2 * 0.5;
}
int main(void) {
    double a[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    printf("%g\n", f(a, 8, 0.0, 0.0));
    return 0;
}
