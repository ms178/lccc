/*
 * AVX2 strict computed-reciprocal lane-pack stress. The first four-lane
 * vector contains both negative and positive denominators (j=2 is negative,
 * j=3 positive), and a nonzero bias arrives as an argument rather than as a
 * compile-time literal. All denominators are nonzero: 7*j - 18 + bias cannot
 * be zero for 0 <= bias < 4. The scalar volatile reference is deliberately
 * NOT eligible for loop vectorization.
 *
 * Test lengths -2..32 exercise zero trips, every scalar remainder 0..3 and
 * repeated vector chunks. This is an ordinary arithmetic pattern, not a
 * workload/function-name exception in the compiler.
 */
#include <stdio.h>

__attribute__((noinline))
static double packed_sum(const double *values, int n, int bias)
{
    double sum = 0.0;
    for (int j = 0; j < n; ++j) {
        int denominator = 7 * j - 18 + bias;
        sum += (1.0 / (double)denominator) * values[j];
    }
    return sum;
}

__attribute__((noinline))
static double scalar_reference(const double *values, int n, int bias)
{
    volatile double sum = 0.0;
    for (int j = 0; j < n; ++j) {
        int denominator = 7 * j - 18 + bias;
        volatile double reciprocal = 1.0 / (double)denominator;
        volatile double product = reciprocal * values[j];
        sum += product;
    }
    return sum;
}

int main(void)
{
    double values[32];
    unsigned int checked = 0;
    for (int bias = 0; bias < 4; ++bias) {
        for (int j = 0; j < 32; ++j)
            values[j] = ((j * 17 + bias * 13) % 101 - 50) * 0.125;
        for (int n = -2; n <= 32; ++n) {
            double got = packed_sum(values, n, bias);
            double reference = scalar_reference(values, n, bias);
            double error = got - reference;
            double magnitude = reference < 0 ? -reference : reference;
            if (error < 0) error = -error;
            if (error > 1e-12 * (1.0 + magnitude)) {
                printf("mismatch: bias=%d n=%d got=%.17g ref=%.17g\n",
                       bias, n, got, reference);
                return 1;
            }
            ++checked;
        }
    }
    printf("strict reciprocal pack: %u cases OK\n", checked);
    return 0;
}
