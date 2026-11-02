/* N=17: forces remainder after quad-FMA vectorized path */
#define N 17
static double A[N][N], B[N][N], C[N][N], R[N][N];
void matmul(void) {
    for (int i = 0; i < N; i++)
        for (int k = 0; k < N; k++)
            for (int j = 0; j < N; j++)
                C[i][j] += A[i][k] * B[k][j];
}
int main(void) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) {
            A[i][j] = (double)(i + j + 1) * 0.5;
            B[i][j] = (double)(i + 1) * (j + 1) * 0.25;
            C[i][j] = R[i][j] = 0;
        }
    matmul();
    for (int i = 0; i < N; i++)
        for (int k = 0; k < N; k++)
            for (int j = 0; j < N; j++)
                R[i][j] += A[i][k] * B[k][j];
    for (int i = 0; i < N; i++)
        for (int j = 0; j < N; j++) {
            double d = C[i][j] - R[i][j];
            if (d < -1e-5 || d > 1e-5) return 1;
        }
    return 0;
}
