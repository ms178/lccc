/* gzip/zlib longest_match compare loop: early-exit byte comparison.
 * Exercises branchy loop codegen and load scheduling. */
#include "bench.h"

#define N 512
static unsigned char a[N], b[N];

void bench_setup(void) {
    for (int i = 0; i < N; i++) { a[i] = (unsigned char)(i * 17); b[i] = a[i]; }
    b[N - 3] ^= 0xff;   /* force a late mismatch */
}

static int match_len(const unsigned char *x, const unsigned char *y, int max) {
    int n = 0;
    while (n < max && x[n] == y[n]) n++;
    return n;
}

unsigned long long bench_run(void) {
    unsigned long long s = 0;
    for (int off = 0; off < 64; off++) s += (unsigned) match_len(a + off, b + off, N - off);
    return s;
}
