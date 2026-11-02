/* Cache-resident per-element multiply/add; independent lanes, strict FP.
   The same source exercises F32/F64, XMM/YMM without ISA-specific dispatch. */
#include <stdint.h>
#include <stdio.h>
#ifdef FP_MAP_DOUBLE
typedef double real;
typedef uint64_t word;
#else
typedef float real;
typedef uint32_t word;
#endif
#ifndef FP_MAP_PASSES
#define FP_MAP_PASSES 4194304
#endif
#define N 512
static real a[N], b[N], c[N], out[N];
__attribute__((noinline))
static void fp_product_map(real *restrict d, const real *restrict x,
                           const real *restrict y, const real *restrict z, int n) {
    for (int i = 0; i < n; ++i) d[i] = x[i] * y[i] + z[i];
}
int main(void) {
    for (int i = 0; i < N; ++i) {
        a[i] = (real)(i - 256) * (real)0.25;
        b[i] = (real)(i % 71 - 35) * (real)0.125;
        c[i] = (real)(i % 29 - 14) * (real)0.0625;
    }
    for (unsigned pass = 0; pass < FP_MAP_PASSES; ++pass) {
        a[pass & (N - 1)] = (real)(pass & 1023) * (real)0.25;
        fp_product_map(out, a, b, c, N);
    }
    uint64_t hash = 1469598103934665603ull;
    for (int i = 0; i < N; ++i) {
        union { real value; word bits; } u;
        u.value = out[i];
        hash = (hash ^ u.bits) * 1099511628211ull;
    }
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
