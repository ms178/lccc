/* Aggregate copies into and within a __seg_gs array: 32-byte struct elements,
 * a generic-to-segment copy and a segment-to-segment copy per iteration. */
#include "bench.h"
#include "seg_gs.h"

#define N 256
struct seg_blk { long v[4]; };
/* 32-byte alignment: GCC 14.2 emits aligned SSE moves for 16-byte-or-larger
 * segment aggregates, so an 8-byte-aligned array faults under -O3. */
struct seg_blk __seg_gs bench_seg_src[N] __attribute__((aligned(32)));
struct seg_blk __seg_gs bench_seg_dst[N] __attribute__((aligned(32)));
extern struct seg_blk bench_seg_src_gen[N] __asm__("bench_seg_src");
extern struct seg_blk bench_seg_dst_gen[N] __asm__("bench_seg_dst");

static struct seg_blk gen_blk[N];

void bench_setup(void) {
    seg_gs_map(bench_seg_src_gen, sizeof(struct seg_blk) * N);
    seg_gs_map(bench_seg_dst_gen, sizeof(struct seg_blk) * N);
    seg_gs_enable();
    for (int i = 0; i < N; i++)
        for (int j = 0; j < 4; j++) gen_blk[i].v[j] = (long) i * 4 + j;
}

__attribute__((noinline)) static void seg_copy_pass(long k) {
    for (int i = 0; i < N; i++) {
        bench_seg_dst[i] = bench_seg_src[i];
        gen_blk[i].v[0] += k;
        bench_seg_src[i] = gen_blk[i];
    }
}

unsigned long long bench_run(void) {
    static long round;
    seg_copy_pass(round++);
    /* Read the result back through the segment (two loads), not a loop. */
    return (unsigned long long) bench_seg_dst[N - 1].v[0] + (unsigned long long) bench_seg_dst[0].v[3];
}
