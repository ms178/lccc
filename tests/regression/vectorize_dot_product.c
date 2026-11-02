/* Regression: reduction dot-product vectorization.
 * The "scale array indexing" step had a `break` that scaled only the FIRST
 * matching GEP per block, leaving the second array of a dot product at scalar
 * stride, so the loop multiplied against the wrong elements (garbage sums). */
#include <stdio.h>

double a[256], b[256];

static double dot(double *x, double *y, int n) {
    double s = 0.0;
    for (int i = 0; i < n; i++)
        s += x[i] * y[i];
    return s;
}

int main(void) {
    for (int i = 0; i < 256; i++) { a[i] = (double)(i + 1); b[i] = (double)(i + 1); }
    static const int ns[] = {1, 2, 3, 4, 5, 7, 8, 255, 256};
    for (unsigned k = 0; k < sizeof(ns) / sizeof(ns[0]); k++) {
        long long n = ns[k];
        double expect = (double)(n * (n + 1) * (2 * n + 1)) / 6.0; /* sum (i+1)^2 */
        double got = dot(a, b, (int)n);
        if (got != expect) {
            printf("FAIL n=%lld expect=%.0f got=%.0f\n", n, expect, got);
            return 1;
        }
    }
    printf("OK\n");
    return 0;
}
