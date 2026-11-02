extern int printf(const char *, ...);

#define N 4

__attribute__((noinline)) static void matmul(
    double c[N][N],
    const double a[N][N],
    const double b[N][N]
) {
    /* In i-j-k order, b[k][j] is a column walk with a 32-byte stride. It must
       never be treated as a contiguous F64 vector stream. */
    for (int i = 0; i < N; ++i) {
        for (int j = 0; j < N; ++j) {
            c[i][j] = 0.0;
            for (int k = 0; k < N; ++k) {
                c[i][j] += a[i][k] * b[k][j];
            }
        }
    }
}

int main(void) {
    const double a[N][N] = {
        {1, 2, 3, 4}, {5, 6, 7, 8},
        {9, 10, 11, 12}, {13, 14, 15, 16}
    };
    const double b[N][N] = {
        {16, 15, 14, 13}, {12, 11, 10, 9},
        {8, 7, 6, 5}, {4, 3, 2, 1}
    };
    double c[N][N];
    matmul(c, a, b);
    for (int i = 0; i < N; ++i) {
        for (int j = 0; j < N; ++j) {
            printf("%.0f%c", c[i][j], j + 1 == N ? '\n' : ' ');
        }
    }
    return 0;
}
