/*
 * OP-26 regression: a tiny static wrapper must not hide a clonable loop body
 * from normal -O2 inlining policy.
 *
 * `wrapper` is called twice.  Its raw IR has only a call and return, so the
 * old unconditional tiny-callee path inlined it into main.  The next rounds
 * then inlined loop_kernel twice, creating two large hot CFGs in main and
 * forcing the simple backend to spill their state.  Keeping the multi-site
 * wrapper outlined lets loop_kernel still inline once into its sole owner,
 * while main pays two cold-boundary calls instead of duplicating the loop.
 */
#include <stdio.h>

static unsigned loop_kernel(unsigned x) {
    for (unsigned i = 0; i < 257; ++i)
        x = x * 1664525u + i + 1013904223u;
    return x;
}

static unsigned wrapper(unsigned x) {
    return loop_kernel(x);
}

/* Prevent IPCP from replacing both calls with constants: this test measures
 * the inliner's call-graph decision, not constant folding. */
volatile unsigned wrapper_seed = 7;

int main(void) {
    unsigned a = wrapper(wrapper_seed);
    unsigned b = wrapper(wrapper_seed + 4);
    printf("%u\n", a ^ b);
    return (a ^ b) == 1843832916u ? 0 : 1;
}
