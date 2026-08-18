/*
 * Front-end-sensitive scalar FP memory-fold benchmark.
 * Derived from p39_stencil5_f32 in patterns/simd_fp_oracle.c.
 */
#include <stdio.h>
#include <stdlib.h>

#define N 65536

static float input[N];
static float output[N];

__attribute__((noinline)) static void stencil5(float *restrict dst,
                                                const float *restrict src,
                                                int n) {
    for (int i = 2; i + 2 < n; ++i)
        dst[i] = src[i - 2] + src[i - 1] + src[i] + src[i + 1] + src[i + 2];
}

int main(int argc, char **argv) {
    int repetitions = argc > 1 ? atoi(argv[1]) : 1000;
    for (int i = 0; i < N; ++i)
        input[i] = (float)(i & 15) * 0.25f;
    for (int i = 0; i < repetitions; ++i)
        stencil5(output, input, N);

    double checksum = 0.0;
    for (int i = 0; i < N; ++i)
        checksum += output[i];
    printf("%.0f\n", checksum);
    return checksum > 600000.0 && checksum < 620000.0 ? 0 : 1;
}
