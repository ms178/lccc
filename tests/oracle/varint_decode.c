/* Self-contained oracle input for the SQLite varint decode loop.
 *
 * Identical control flow and identical datatypes to tests/bench/k_varint.c;
 * only the harness (bench.h / bench_setup / bench_run) is removed so the file
 * can be compiled on Compiler Explorer without a second translation unit.
 * The function under measurement is `bench_run`, which is the name the
 * oracle scripts filter on.
 */
#include <stdint.h>

#define N 2048

/* External linkage ON PURPOSE.
 *
 * The kernel's `v` is `static` and is filled by `bench_setup`, so the real
 * benchmark never knows its contents. Reproducing it here as a `static`
 * array with no initialiser makes it provably all-zero, and clang then folds
 * the entire decode to a constant -- the oracle reports 2 instructions for a
 * function the benchmark spends its life in. Giving it external linkage makes
 * the contents equally unknown to every compiler, which is the property the
 * comparison actually needs. Verified: with `static`, clang emits
 * `movl $2044,%eax; retq`.
 */
unsigned char v[N];

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
