/* SQLite varint decode: short serial-dependent branchy chain.
 * Exercises branch layout and the cost of mispredicted early exits. */
#include <stdint.h>
#include "bench.h"

#define N 2048
static unsigned char v[N];

void bench_setup(void) {
    for (int i = 0; i < N; i++) v[i] = (unsigned char) ((i * 37) | ((i & 3) ? 0x80 : 0));
}

static int get_varint32(const unsigned char *p, uint32_t *out) {
    uint32_t a = p[0];
    if (a < 0x80) { *out = a; return 1; }
    a = (a & 0x7f) << 7;
    uint32_t b = p[1];
    if (b < 0x80) { *out = a | b; return 2; }
    a = (a | (b & 0x7f)) << 7;
    uint32_t c = p[2];
    if (c < 0x80) { *out = a | c; return 3; }
    *out = a | (c & 0x7f);
    return 4;
}

unsigned long long bench_run(void) {
    unsigned long long acc = 0;
    uint32_t out;
    for (int i = 0; i + 4 < N; i++) { acc += (unsigned) get_varint32(v + i, &out) + out; }
    return acc;
}
