/* zlib-ng adler32 inner loop: two dependent accumulators over bytes.
 * Exercises loop-carried reduction, unrolling and accumulator reassociation. */
#include <stddef.h>
#include <stdint.h>
#include "bench.h"

#define N 4096
static unsigned char buf[N];

void bench_setup(void) {
    for (int i = 0; i < N; i++) buf[i] = (unsigned char) (i * 31 + 7);
}

static uint32_t adler32_chunk(uint32_t adler, const unsigned char *b, size_t len) {
    uint32_t s1 = adler & 0xffff, s2 = adler >> 16;
    for (size_t i = 0; i < len; i++) { s1 += b[i]; s2 += s1; }
    return (s1 % 65521u) | ((s2 % 65521u) << 16);
}

unsigned long long bench_run(void) { return adler32_chunk(1, buf, N); }
