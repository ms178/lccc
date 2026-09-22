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
 * Freestanding: the failure signal is a real SIGABRT raised through the i386
 * syscall path rather than libc's abort(), so the test links with -nostdlib
 * and needs no 32-bit libc on the host. PIC is retained — it is what makes
 * the GOT base live in %ebx, which is the whole point of the test.
 *
 * Proven: dies with SIGABRT when the NonlocalGoto pool-clear is reverted,
 * passes with it.
 */
int g = 10;

/* i386 Linux syscalls, entered directly: no libc, no 32-bit headers. */
static void sys_exit(int status)
{
	__asm__ volatile("int $0x80" : : "a"(1), "b"(status) : "memory");
	for (;;)
		;
}

static void sys_abort(void)
{
	int pid;

	__asm__ volatile("int $0x80" : "=a"(pid) : "a"(20) : "memory");
	/* kill(pid, SIGABRT) — the same termination the libc abort() gives,
	 * so a revert still shows up as SIGABRT rather than a plain exit. */
	__asm__ volatile("int $0x80" : : "a"(37), "b"(pid), "c"(6) : "memory");
	sys_exit(134);
}

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
        sys_abort();
    if (exercise(2, 0) != 14)
        sys_abort();
    return 0;
}

void _start(void)
{
	sys_exit(main());
}
