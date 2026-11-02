/* __seg_gs array reduction: one %gs-relative load per element, indexed by the
 * induction variable. The hot loop is the whole kernel. */
#include "bench.h"
#include "seg_gs.h"

#define N 1024
long __seg_gs bench_seg_sum_arr[N];
extern long bench_seg_sum_gen[N] __asm__("bench_seg_sum_arr");

void bench_setup(void) {
    seg_gs_map(bench_seg_sum_gen, sizeof(long) * N);
    seg_gs_enable();
    long *mir = (long *) SEG_GS_MIRROR(bench_seg_sum_gen);
    for (int i = 0; i < N; i++) mir[i] = (long) ((unsigned) i * 2654435761u & 0xffffu);
}

__attribute__((noinline)) static long seg_sum(void) {
    long s = 0;
    for (int i = 0; i < N; i++) s += bench_seg_sum_arr[i];
    return s;
}

unsigned long long bench_run(void) {
    return (unsigned long long) seg_sum();
}
