/* i686 twin of nested_nonlocal_goto_callee_saved: a nested non-local goto
 * bypasses the child's epilogue, so the child allocator must not use
 * callee-saved GPR homes (the i686 prologue must clear the pool when the
 * function contains a NonlocalGoto, mirroring x86-64).
 *
 * Exposure mechanism (32-bit PIC): the parent holds the GOT base in
 * callee-saved %ebx across the nested call and reads a global through it
 * after the goto target. Without the pool clear the child takes %ebx as a
 * home; the goto bypasses the child's pop, and the parent dereferences the
 * GOT through the child's garbage %ebx. Requires PIC (no -fno-pic).
 *
 * Proven: aborts with the NonlocalGoto pool-clear reverted (SIGABRT),
 * passes with it.
 */
extern void abort(void);

int g = 10;

__attribute__((noinline))
static int exercise(int a, int jump) {
    __label__ target;

    void transfer(int enabled) {
        if (enabled)
            goto target;
    }

    int carried = a + 2;
    transfer(jump);
target:
    return carried + g;
}

int main(void) {
    if (exercise(1, 1) != 13)
        abort();
    if (exercise(2, 0) != 14)
        abort();
    return 0;
}
