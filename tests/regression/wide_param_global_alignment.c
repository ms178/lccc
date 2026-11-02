/* A noinline vector kernel receives globals across a call boundary.  Odd-sized
   arrays make the ordinary 16-byte policy leave some bases 16 modulo 32. */
#include <stdint.h>
#include <stdio.h>

#define N 257
static double a[N], b[N], c[N], out[N];

__attribute__((noinline))
static void vector_kernel(double *restrict dst, const double *restrict x,
                          const double *restrict y, const double *restrict z,
                          int n) {
    for (int i = 0; i < n; ++i)
        dst[i] = x[i] * y[i] + z[i];
}

int main(void) {
    for (int i = 0; i < N; ++i) {
        a[i] = (double)(i % 17 - 8) * 0.5;
        b[i] = (double)(i % 23 - 11) * 0.25;
        c[i] = (double)(i % 13 - 6) * 0.125;
    }
    vector_kernel(out, a, b, c, N);
    for (int i = 0; i < N; ++i) {
        double expected = (double)(i % 17 - 8) * 0.5
                        * ((double)(i % 23 - 11) * 0.25)
                        + (double)(i % 13 - 6) * 0.125;
        if (out[i] != expected) return 1;
    }
    if (((uintptr_t)a | (uintptr_t)b | (uintptr_t)c | (uintptr_t)out) & 31u)
        return 2;
    puts("wide_param_global_alignment: PASS");
    return 0;
}
