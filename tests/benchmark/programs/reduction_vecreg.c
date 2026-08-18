/*
 * Register-resident auto-vectorized F32 reduction benchmark.
 * The 65,536-element sum and dot loops exercise vector accumulators while
 * keeping the input set stable across paired treatment/control runs.
 */
#include <stdio.h>
#include <stdlib.h>

#define N 65536

static float input_a[N];
static float input_b[N];
static volatile float sink;

__attribute__((noinline)) static float sum_f32(const float *restrict values, int n) {
    float sum = 0.0f;
    for (int i = 0; i < n; ++i)
        sum += values[i];
    return sum;
}

__attribute__((noinline)) static float dot_f32(const float *restrict a,
                                               const float *restrict b,
                                               int n) {
    float sum = 0.0f;
    for (int i = 0; i < n; ++i)
        sum += a[i] * b[i];
    return sum;
}

int main(int argc, char **argv) {
    int repetitions = argc > 1 ? atoi(argv[1]) : 5000;
    for (int i = 0; i < N; ++i) {
        input_a[i] = (float)(i & 15) * 0.25f;
        input_b[i] = (float)(i & 7) * 0.125f;
    }
    for (int i = 0; i < repetitions; ++i) {
        sink = sum_f32(input_a, N);
        sink += dot_f32(input_a, input_b, N);
    }
    printf("%.0f\n", (double)sink);
    return sink == 187392.0f ? 0 : 1;
}
