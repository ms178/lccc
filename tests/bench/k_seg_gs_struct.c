/* Struct fields through a __seg_gs pointer that lives in generic memory (the
 * per-CPU idiom: load the segment pointer, then %gs-relative field accesses).
 * Each element updates two fields and reads a third. */
#include "bench.h"
#include "seg_gs.h"

#define N 256
struct seg_cell { long a, b, c, d; };
struct seg_cell __seg_gs bench_seg_cells[N];
extern struct seg_cell bench_seg_cells_gen[N] __asm__("bench_seg_cells");

/* Generic-memory holder of a segment pointer. */
static struct seg_cell __seg_gs *cursor;

void bench_setup(void) {
    seg_gs_map(bench_seg_cells_gen, sizeof(struct seg_cell) * N);
    seg_gs_enable();
    struct seg_cell *mir = (struct seg_cell *) SEG_GS_MIRROR(bench_seg_cells_gen);
    for (int i = 0; i < N; i++) {
        mir[i].a = i;
        mir[i].b = 2 * i + 1;
        mir[i].c = 0;
        mir[i].d = (long) i ^ 0x5a;
    }
    cursor = &bench_seg_cells[0];
}

__attribute__((noinline)) static long seg_struct_pass(void) {
    struct seg_cell __seg_gs *p = cursor;
    long acc = 0;
    for (int i = 0; i < N; i++) {
        p[i].a += p[i].b;
        p[i].c = p[i].a ^ p[i].d;
        acc += p[i].c;
    }
    return acc;
}

unsigned long long bench_run(void) {
    return (unsigned long long) seg_struct_pass();
}
