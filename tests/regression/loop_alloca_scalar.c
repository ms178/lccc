/* allocas whose single use-block is inside a LOOP must not
 * be placed in the shared block-local slot region. That region reuses
 * offsets across blocks under the assumption of exclusive execution; a loop
 * body re-enters, so a loop-carried alloca (written in iteration N, read in
 * N+1) is clobbered by another block's value at the same offset.
 * (vector_defer_multidef_slot: adler32's loop-body temps and loop-carried
 * slots aliased across the wide/narrow loop blocks.)
 * Scalar shape: two loops, each with a block-local array alloca, sharing
 * the block-local region; loop 1's array must survive into loop 2. */
#include <stdio.h>

static int two_loops(int n) {
    int acc = 0;
    for (int i = 0; i < n; i++) {
        int a[2] = {i, i + 1};          /* block-local alloca in loop 1 */
        acc += a[0] + a[1];
    }
    for (int i = 0; i < n; i++) {
        int b[2] = {2 * i, 2 * i + 1};  /* block-local alloca in loop 2 */
        acc += b[0] + b[1];
    }
    return acc;
}

int main(void) {
    int r = two_loops(10);   /* loop1: 100, loop2: 190 -> 290 */
    if (r != 290) { printf("FAIL r=%d\n", r); return 1; }
    printf("PASS loop_alloca_scalar\n");
    return 0;
}
