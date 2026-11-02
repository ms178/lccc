/* Regression: matmul vectorization element-type gate.
 * The matmul transform lowered to packed-DOUBLE FMA with no check on the
 * element type; a float matmul was reinterpreted as doubles (wrong stride)
 * and segfaulted. Float/int matmuls must be left scalar. */
#include <stdio.h>

static float A[64][64], B[64][64], C[64][64];

static float fmatmul_checksum(int n) {
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++) {
            A[i][j] = 1.0f; B[i][j] = 1.0f; C[i][j] = 0.0f;
        }
    for (int i = 0; i < n; i++)
        for (int k = 0; k < n; k++)
            for (int j = 0; j < n; j++)
                C[i][j] += A[i][k] * B[k][j];
    float s = 0.0f;
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++)
            s += C[i][j];
    return s;
}

int main(void) {
    static const int ns[] = {1, 2, 3, 4, 5, 7, 8, 15, 16, 17, 63, 64};
    for (unsigned k = 0; k < sizeof(ns) / sizeof(ns[0]); k++) {
        int n = ns[k];
        float expect = (float)n * n * n;
        float got = fmatmul_checksum(n);
        if (got != expect) {
            printf("FAIL n=%d expect=%.0f got=%.0f\n", n, (double)expect, (double)got);
            return 1;
        }
    }
    printf("OK\n");
    return 0;
}
