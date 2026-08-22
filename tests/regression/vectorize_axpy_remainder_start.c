// Regression: insert_remainder_loop must derive the scalar-remainder start
// index from the vector loop's IV representation. The two-wide hoisted
// FmaF64x2 scheme (AArch64 NEON pair / LCCC_FORCE_SSE2) uses a GROUP index
// stepping 1 with 4 doubles per group -> start = IV*4. The old byte-offset
// formula (IV >> 3) restarted the remainder at element 0 and re-accumulated
// the whole row: matmul C[i][j] came out exactly 2x too large on AArch64 -O2.
int printf(const char *, ...);
#define N 8
static double A[N][N], B[N][N], C[N][N];
void matmul(void) {
    for (int i = 0; i < N; i++)
        for (int k = 0; k < N; k++)
            for (int j = 0; j < N; j++)
                C[i][j] += A[i][k] * B[k][j];
}
int main(void) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) {
            A[i][j] = (double)(i + j) / N;
            B[i][j] = (double)(i * j + 1) / N;
        }
    matmul();
    printf("%.6f %.6f %.6f\n", C[0][0], C[3][6], C[7][7]);
    return 0;
}
