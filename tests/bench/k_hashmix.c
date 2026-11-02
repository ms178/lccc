/* FNV-1a over a small key set: integer multiply/xor dependency chain.
 * Exercises scalar ALU scheduling and constant multiply lowering. */
#include <stdint.h>
#include <stddef.h>
#include "bench.h"

#define N 256
static unsigned char keys[N][16];

void bench_setup(void) {
    for (int i = 0; i < N; i++)
        for (int j = 0; j < 16; j++) keys[i][j] = (unsigned char) (i * 7 + j * 13);
}

static uint64_t fnv1a(const unsigned char *p, size_t n) {
    uint64_t h = 1469598103934665603ull;
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 1099511628211ull; }
    return h;
}

unsigned long long bench_run(void) {
    uint64_t acc = 0;
    for (int i = 0; i < N; i++) acc ^= fnv1a(keys[i], 16);
    return acc;
}
