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
/* Narrow loads exercise the integer-promotion Cast tail on ILP32 AND LP64
 * backends: I8 -> I32 sign extension and U16 -> I32 zero extension. */
static signed char C[4] = { -1, -2, -3, -4 };
static signed char D[4] = { 5, 6, 7, 8 };
static unsigned short E[4] = { 100, 200, 300, 400 };
static unsigned short F[4] = { 500, 600, 700, 800 };
static volatile int g_cond;

__attribute__((noinline)) static int pick(const int *p, const int *q) {
    return g_cond ? p[2] : q[2];
}

__attribute__((noinline)) static signed char pick8(const signed char *p,
                                                   const signed char *q) {
    return g_cond ? p[2] : q[2];
}

__attribute__((noinline)) static unsigned short pick16(const unsigned short *p,
                                                      const unsigned short *q) {
    return g_cond ? p[2] : q[2];
}

int lccc_main(void) {
    g_cond = 1;
    int x = pick(A, B);              /* 33 */
    signed char c = pick8(C, D);    /* -3 */
    unsigned short e = pick16(E, F);/* 300 */
    g_cond = 0;
    int y = pick(A, B);              /* 77 */
    signed char d = pick8(C, D);     /* 7 */
    unsigned short f = pick16(E, F); /* 700 */
    return (x * 3 + y * 7 + c * 11 + d * 17 + (int)e * 13 + (int)f * 19) % 61;
}
