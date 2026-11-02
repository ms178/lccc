/* Stronger correctness oracle for the unmodified benchmark: check every cell.
 * All factors are dyadic and the integer numerators fit in 53 bits, so the
 * closed form is exact with or without contraction. Do not use non-power-of-two N.
 */
#include <stdint.h>
#ifndef MATMUL_TEST_N
#define MATMUL_TEST_N 32
#endif
#if MATMUL_TEST_N < 1 || MATMUL_TEST_N > 256 || (MATMUL_TEST_N & (MATMUL_TEST_N - 1))
#error MATMUL_TEST_N must be a power of two from 1 through 256
#endif
#define N MATMUL_TEST_N
#define main benchmark_main
#include "../benchmark/programs/matmul.c"
#undef main
int main(void) {
    if (benchmark_main() != 0) return 1;
    const uint64_t sum_k = (uint64_t)N * (N - 1) / 2;
    const uint64_t sum_k2 = (uint64_t)(N - 1) * N * (2 * N - 1) / 6;
    for (int i = 0; i < N; ++i) {
        for (int j = 0; j < N; ++j) {
            uint64_t numerator = (uint64_t)i * j * sum_k + (uint64_t)i * N
                               + (uint64_t)j * sum_k2 + sum_k;
            double expected = (double)numerator / ((double)N * N);
            if (C[i][j] != expected) {
                printf("mismatch at %d,%d: %.17g != %.17g\n", i, j, C[i][j], expected);
                return 2;
            }
        }
    }
    puts("matmul_full_oracle: PASS");
    return 0;
}
