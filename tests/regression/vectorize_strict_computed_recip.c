/*
 * PERF-41: four independent computed denominators may execute in parallel,
 * but their floating-point products must feed the scalar accumulator in
 * source order.  A horizontal tree would make the n=4 case return 2.0:
 *
 *   ((0 + 1e100) + 1) + -1e100 + 1 == 1
 *
 * The 1 << j denominator is intentionally non-constant, forcing the narrow
 * scalar-I32 DAG cloning path rather than an ordinary contiguous reduction.
 * Calls keep j in [0, 4], so the C shift is defined.
 */
#include <stdio.h>

__attribute__((noinline))
static double ordered_recip_sum(const double *values, int n) {
    double sum = 0.0;
    for (int j = 0; j < n; ++j) {
        int denominator = 1 << j;
        sum += (1.0 / (double)denominator) * values[j];
    }
    return sum;
}

int main(void) {
    static const double values[] = {
        1.0e100, 2.0, -4.0e100, 8.0, 16.0,
    };
    const double n0 = ordered_recip_sum(values, 0);
    const double neg = ordered_recip_sum(values, -3);
    const double n3 = ordered_recip_sum(values, 3);
    const double n4 = ordered_recip_sum(values, 4);
    const double n5 = ordered_recip_sum(values, 5);

    printf("%.17g %.17g %.17g %.17g %.17g\n", n0, neg, n3, n4, n5);
    return n0 == 0.0 && neg == 0.0 && n3 == 0.0 && n4 == 1.0 && n5 == 2.0
               ? 0
               : 1;
}
