/* Freestanding four-ISA execution fixture for SV-03 conditional-load sinking.
 *
 * The untaken `q`/`p` pointer is a valid but DIFFERENT array, so a transform
 * that speculates the load instead of the address still produces a wrong
 * answer here.  `g_cond` is volatile and is set at runtime, so the diamond
 * cannot be folded by the frontend.  `pick` is noinline so the diamond really
 * exists as a function body on every backend.
 *
 * No libc, no headers: the harness links a per-ISA `_start` and executes the
 * exit status natively (x86-64) or under qemu-user (i686/AArch64/RISC-V64).
 */
static int A[4] = { 11, 22, 33, 44 };
static int B[4] = { 55, 66, 77, 88 };
static volatile int g_cond;

__attribute__((noinline)) static int pick(const int *p, const int *q) {
    return g_cond ? p[2] : q[2];
}

int lccc_main(void) {
    g_cond = 1;
    int x = pick(A, B);   /* 33 */
    g_cond = 0;
    int y = pick(A, B);   /* 77 */
    return (x * 3 + y * 7) % 61;
}
