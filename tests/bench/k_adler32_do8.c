/* zlib-ng adler32 DO8 inner loop: two dependent accumulators, SOURCE-unrolled
 * by eight. This is the shape scripts/../src/passes/reassoc_accum.rs targets
 * ("ICC's Adler-32 chain break"): a serial `s1 += b[i]; s2 += s1;` chain of
 * N >= 4 steps in one basic block, fed by independent byte loads.
 *
 * The non-unrolled k_adler32.c never exercises that pass (its chain is one
 * step long in the IR), so this kernel is the one that must be measured
 * whenever the reassociation's cost model is touched. Session-30 data
 * (1 MiB x 30 reps, -O2, this kernel's DO8 body):
 *
 *     reassoc_accum forced ON   106 instructions, 18.7 ms
 *     cost model (default)       70 instructions, 11.1 ms
 *     gcc -O2                    65 instructions, 10.3 ms
 *
 * i.e. the closed form is a ~1.7x slowdown on its own flagship pattern: the
 * loop is issue-throughput-bound, not recurrence-bound, because both phis
 * advance by a single 1-cycle add and an out-of-order core overlaps
 * iterations. Run with:
 *     scripts/bench_kernels.py --kernels adler32_do8 --flags=-O2
 *     CCC_REASSOC_ACCUM_FORCE=1 scripts/bench_kernels.py --kernels adler32_do8
 */
#include <stddef.h>
#include <stdint.h>

#include "bench.h"

#define N 4096
static unsigned char buf[N];

void bench_setup(void) {
    for (int i = 0; i < N; i++) buf[i] = (unsigned char) (i * 31 + 7);
}

static uint32_t adler32_do8(uint32_t adler, const unsigned char *p, size_t len) {
    uint32_t s1 = adler & 0xffff;
    uint32_t s2 = adler >> 16;
    while (len >= 8) {
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        s1 += *p++; s2 += s1;
        len -= 8;
    }
    while (len--) { s1 += *p++; s2 += s1; }
    return (s2 << 16) | s1;
}

unsigned long long bench_run(void) { return adler32_do8(1, buf, N); }
