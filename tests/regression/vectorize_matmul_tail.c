/* Regression: AVX2 matmul vectorization tail (N % 4 != 0).
 * The byte limit of the vector loop used to be N*8, running ceil(N/4)
 * iterations: the last iteration wrote past the logical end of the row
 * (clobbering a sentinel canary) and the scalar remainder never fired.
 * Correct: floor(N/4) vector iterations + scalar remainder.
 *
 * This also covers the fixed-scratch register bug exposed when that remainder
 * transition changed from signed division to a logical shift: division had
 * accidentally kept the byte IV out of RDX, while the packed-FMA emitter
 * overwrote RDX. At n=17 the corrupted IV skipped the final scalar element. */
#include <stdio.h>

static double A[256][256], B[256][256], C[256][256];

static int matmul_verify(int n) {
    for (int i = 0; i < 256; i++)
        for (int j = 0; j < 256; j++) {
            A[i][j] = 1.0; B[i][j] = 1.0; C[i][j] = 999.0; /* canary */
        }
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++)
            C[i][j] = 0.0;
    for (int i = 0; i < n; i++)
        for (int k = 0; k < n; k++)
            for (int j = 0; j < n; j++)
                C[i][j] += A[i][k] * B[k][j];
    double s = 0.0;
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            if (C[i][j] != (double)n) { printf("FAIL [%d][%d]=%.0f\n", i, j, C[i][j]); return 0; }
            s += C[i][j];
        }
        /* column n must be untouched (vector loop ran past the row end).
         * Only valid when n < 256: for n == 256 the whole row is in range. */
        if (n < 256 && C[i][n] != 999.0) { printf("FAIL canary [%d][%d]=%.0f\n", i, n, C[i][n]); return 0; }
    }
    if (s != (double)n * n * n) { printf("FAIL sum=%.0f\n", s); return 0; }
    return 1;
}

int main(void) {
    static const int ns[] = {
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9,
        15, 16, 17, 31, 32, 33, 63, 64, 65, 255, 256
    };
    for (unsigned k = 0; k < sizeof(ns) / sizeof(ns[0]); k++)
        if (!matmul_verify(ns[k])) return 1;
    printf("OK\n");
    return 0;
}
