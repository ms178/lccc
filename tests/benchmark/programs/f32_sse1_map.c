/* Original cache-resident strict-FP throughput kernel. Byte hashing keeps
   the harness itself legal without SSE2 (no MOVD float/integer bitcasts). */
#include <stdint.h>
#include <stdio.h>
#if defined(SSE1_MAP_REQUIRE_ONLY) && !defined(__SSE__)
#error SSE1 is required
#endif
#if defined(SSE1_MAP_REQUIRE_ONLY) && defined(__SSE2__)
#error this workload must exercise an SSE1-only target
#endif
#ifndef SSE1_MAP_PASSES
#define SSE1_MAP_PASSES 4194304
#endif
#define N 512
static float a[N], b[N], c[N], out[N];
__attribute__((noinline))
static void sse1_map(float *restrict d, const float *restrict x,
                     const float *restrict y, const float *restrict z, int n) {
    for (int i = 0; i < n; ++i) d[i] = x[i] * y[i] + z[i];
}
int main(void) {
    for (int i = 0; i < N; ++i) {
        a[i] = (float)(i - 256) * 0.25f;
        b[i] = (float)(i % 71 - 35) * 0.125f;
        c[i] = (float)(i % 29 - 14) * 0.0625f;
    }
    for (unsigned pass = 0; pass < SSE1_MAP_PASSES; ++pass) {
        a[pass & (N - 1)] = (float)(pass & 1023) * 0.25f;
        sse1_map(out, a, b, c, N);
    }
    uint64_t hash = 1469598103934665603ull;
    const unsigned char *bytes = (const unsigned char *)out;
    for (unsigned i = 0; i < sizeof out; ++i)
        hash = (hash ^ bytes[i]) * 1099511628211ull;
    printf("%016llx\n", (unsigned long long)hash);
    return 0;
}
