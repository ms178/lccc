/* __seg_gs array store: one %gs-relative store per element. The value depends
 * on the index and the run count so the stores cannot be hoisted or folded. */
#include "bench.h"
#include "seg_gs.h"

#define N 1024
long __seg_gs bench_seg_fill_arr[N];
extern long bench_seg_fill_gen[N] __asm__("bench_seg_fill_arr");

static unsigned long long rounds;

void bench_setup(void) {
    seg_gs_map(bench_seg_fill_gen, sizeof(long) * N);
    seg_gs_enable();
}

__attribute__((noinline)) static void seg_fill(long k) {
    for (int i = 0; i < N; i++) bench_seg_fill_arr[i] = (long) i * 3 + k;
}

unsigned long long bench_run(void) {
    seg_fill((long) (rounds & 0xffff));
    rounds++;
    /* Read the result back through the segment (two loads), not a loop. */
    return (unsigned long long) bench_seg_fill_arr[N - 1] + (unsigned long long) bench_seg_fill_arr[0];
}
