/* Cache-resident pointwise low-dword products. No ISA-specific source or
   compiler-specific dispatch: run the same workload at SSE2/SSE4.1/AVX2.
   Unsigned wraparound defines every input, product, mutation and checksum. */
#include <stdint.h>
#include <stdio.h>
#ifndef PRODUCT_MAP_PASSES
#define PRODUCT_MAP_PASSES 1048576
#endif
#define N 2048
static uint32_t a[N], b[N], out[N];

__attribute__((noinline))
static void product_map(uint32_t *restrict d, const uint32_t *restrict x,
                        const uint32_t *restrict y, int n) {
    for (int i = 0; i < n; ++i) d[i] = x[i] * y[i];
}

int main(void) {
    uint32_t state = 0x179b3d21u;
    for (int i = 0; i < N; ++i) {
        state = state * 1664525u + 1013904223u;
        a[i] = state;
        state = state * 1664525u + 1013904223u;
        b[i] = state;
    }
    for (unsigned pass = 0; pass < PRODUCT_MAP_PASSES; ++pass) {
        a[pass & (N - 1)] ^= pass;
        product_map(out, a, b, N);
    }
    uint64_t hash = 1469598103934665603ull;
    for (int i = 0; i < N; ++i) hash = (hash ^ out[i]) * 1099511628211ull;
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
