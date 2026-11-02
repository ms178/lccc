/* Regression: I32 reduction vectorization (SSE2 4-wide / AVX2 8-wide add +
 * horizontal reduce). Exercises VecAddI32x{4,8} and the horizontal-add
 * sequences whose legacy SSE2 2-operand instructions (paddd, psrldq) were
 * emitted in invalid 3-operand form. */
#include <stdio.h>

int arr[256];

static int isum(int *x, int n) {
    int s = 0;
    for (int i = 0; i < n; i++)
        s += x[i];
    return s;
}

int main(void) {
    for (int i = 0; i < 256; i++) arr[i] = 1;
    static const int ns[] = {1, 2, 3, 4, 5, 7, 8, 15, 16, 255, 256};
    for (unsigned k = 0; k < sizeof(ns) / sizeof(ns[0]); k++) {
        int n = ns[k];
        int got = isum(arr, n);
        if (got != n) {
            printf("FAIL n=%d expect=%d got=%d\n", n, n, got);
            return 1;
        }
    }
    printf("OK\n");
    return 0;
}
