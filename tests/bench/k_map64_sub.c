/* 64-bit elementwise map with a genuine dependent output.  This fixture
 * measures the x86-64 two-lane XMM lowering against CCC_NO_MAP_VEC=1 in
 * scripts/bench_kernels.py; the mutation keeps successive calls distinct. */
#include <stdint.h>
#include <stddef.h>
#include "bench.h"

#define N 1024
static uint64_t a[N], b[N], d[N];
static unsigned t;

void bench_setup(void) {
    t = 0;
    for (unsigned i = 0; i < N; ++i) {
        a[i] = (uint64_t)i * UINT64_C(0x123456789abcd);
        b[i] = ((uint64_t)i + 7u) * UINT64_C(0x1234567);
    }
}

unsigned long long bench_run(void) {
    for (size_t i = 0; i < N; ++i) d[i] = a[i] - b[i];
    uint64_t sample = d[t & (N - 1)];
    a[t & (N - 1)] += t | 1u;
    ++t;
    return sample;
}
