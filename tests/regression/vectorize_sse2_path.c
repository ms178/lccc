/* Regression: legacy SSE2 vectorization path (LCCC_FORCE_SSE2=1).
 * The SSE2 matmul remainder loop computed j_rem_start = iv/8 on an ELEMENT-
 * index IV (should be iv*2), re-processing the array; FmaF64x2 conflated the
 * B and C pointers (same offset, different base); and the 2×F64/4×I32
 * horizontal-adds plus VecAddI32x4 emitted legacy 2-operand instructions in
 * invalid 3-operand form ("SSE op requires 2 operands"). All must be correct
 * under the forced-SSE2 mode. */
#include <stdio.h>

double a[256], b[256];
int iarr[256];

static double dot(double *x, double *y, int n) {
    double s = 0.0;
    for (int i = 0; i < n; i++) s += x[i] * y[i];
    return s;
}

static int isum(int *x, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += x[i];
    return s;
}

int main(void) {
    for (int i = 0; i < 256; i++) { a[i] = (double)(i + 1); b[i] = (double)(i + 1); iarr[i] = 1; }
    static const int ns[] = {1, 2, 3, 5, 7, 8, 255, 256};
    for (unsigned k = 0; k < sizeof(ns) / sizeof(ns[0]); k++) {
        long long n = ns[k];
        double expect_dot = (double)(n * (n + 1) * (2 * n + 1)) / 6.0;
        if (dot(a, b, (int)n) != expect_dot) { printf("FAIL dot n=%lld\n", n); return 1; }
        if (isum(iarr, (int)n) != (int)n) { printf("FAIL isum n=%lld\n", n); return 1; }
    }
    printf("OK\n");
    return 0;
}
