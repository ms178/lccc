/* Small dynamic reductions for screening vector-loop remainder transitions.
 * Values and totals remain exactly representable in F64, so this doubles as a
 * deterministic correctness check across every AVX2 width boundary exercised.
 */
#include <stdio.h>
#include <stdlib.h>

static double a[65], b[65];

__attribute__((noinline))
static double sum_f64(const double *restrict x, int n) {
    double sum = 0.0;
    for (int i = 0; i < n; ++i)
        sum += x[i];
    return sum;
}

__attribute__((noinline))
static double dot_f64(const double *restrict x, const double *restrict y, int n) {
    double sum = 0.0;
    for (int i = 0; i < n; ++i)
        sum += x[i] * y[i];
    return sum;
}

int main(int argc, char **argv) {
    int repetitions = argc > 1 ? atoi(argv[1]) : 1;
    static const int bounds[] = {
        0, 1, 2, 3, 4, 5, 7, 8, 9,
        15, 16, 17, 31, 32, 33, 63, 64, 65
    };
    for (int i = 0; i < 65; ++i) {
        a[i] = (double)(i + 1);
        b[i] = (double)(i + 2);
    }

    double result = 0.0;
    for (int r = 0; r < repetitions; ++r)
        for (unsigned k = 0; k < sizeof(bounds) / sizeof(bounds[0]); ++k) {
            int n = bounds[k];
            result += sum_f64(a, n);
            result += dot_f64(a, b, n);
        }

    printf("%.0f\n", result);
    return 0;
}
