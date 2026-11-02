#include <stdint.h>
#include <stdio.h>
#ifndef ALIGN_N
#define ALIGN_N 257
#endif
static double wide_a[ALIGN_N], wide_b[ALIGN_N], wide_out[ALIGN_N];
__attribute__((noinline)) static void vector_user(void) {
    for (int i = 0; i < ALIGN_N; ++i)
        wide_out[i] = wide_a[i] + wide_b[i];
}
int main(void) {
    for (int i = 0; i < ALIGN_N; ++i) {
        wide_a[i] = (double)((i % 17) - 8) * 0.5;
        wide_b[i] = (double)((i % 23) - 11) * 0.25;
    }
    vector_user();
    for (int i = 0; i < ALIGN_N; ++i) {
        double expected = (double)((i % 17) - 8) * 0.5
                        + (double)((i % 23) - 11) * 0.25;
        if (wide_out[i] != expected) return 1;
    }
#ifdef CHECK_WIDE_PLACEMENT
    if (((uintptr_t)wide_a | (uintptr_t)wide_b | (uintptr_t)wide_out) & 31u) return 2;
#endif
    puts("wide_global_alignment: PASS");
    return 0;
}
