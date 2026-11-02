/* glibc-style memchr scan: pointer-increment loop with an early exit.
 * Exercises IV strength reduction, SIB addressing and un-IVSR. */
#include <stddef.h>
#include "bench.h"

#define N 8192
static unsigned char buf[N];

void bench_setup(void) {
    for (int i = 0; i < N; i++) buf[i] = (unsigned char) (i | 1);
    buf[N - 17] = 0;
}

static const unsigned char *scan(const unsigned char *p, unsigned char c, size_t n) {
    for (size_t i = 0; i < n; i++) if (p[i] == c) return p + i;
    return 0;
}

unsigned long long bench_run(void) {
    const unsigned char *r = scan(buf, 0, N);
    return r ? (unsigned long long) (r - buf) : 0ull;
}
