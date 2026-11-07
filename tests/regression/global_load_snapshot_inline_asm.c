/*
 * Regression: a captured global load is an SSA snapshot.  A later memory
 * clobbering inline-asm statement may change the global, but it must not
 * retroactively change the value read before the asm.  This caught an unsafe
 * global-load sinking experiment that rematerialized the load after the asm.
 */
#include <stdio.h>

int global_load_snapshot = 1;

__attribute__((noinline))
static int capture_then_asm_write(int choose) {
    int before = global_load_snapshot;
#ifdef __i386__
    /* RIP-relative addressing does not exist in 32-bit mode.  Keep the
     * memory clobber (rather than an output operand) in both variants. */
    __asm__ volatile ("movl $2, global_load_snapshot" ::: "memory");
#else
    __asm__ volatile ("movl $2, global_load_snapshot(%%rip)" ::: "memory");
#endif
    if (choose)
        return before;
    return before + 0;
}

int main(void) {
    int captured = capture_then_asm_write(1);
    printf("%d %d\n", captured, global_load_snapshot);
    return captured != 1 || global_load_snapshot != 2;
}
