/* Freestanding i686 select-differential corpus.
 *
 * WHY THIS EXISTS
 * ---------------
 * The i686 back end lowers an integer `Select` to a branch diamond; the
 * `fold_select_diamond_cmov` peephole (cross-pollinated from the x86-64 cmov
 * select lowering) folds the register-arm diamond into a single `cmovcc`.
 * A cmov fold is only correct if BOTH arms' values and the condition are
 * preserved on every path, so the proof is execution, not assembly inspection:
 * this program computes a 64-bit checksum over hundreds of thousands of
 * pseudo-random select evaluations and writes the raw 8 bytes to stdout. The
 * runner compares lccc's bytes against GCC's at several -O levels, and against
 * lccc with the fold disabled (CCC_NO_I686_SELECT_CMOV), so a miscompile shows
 * up as a byte mismatch rather than a silent wrong answer.
 *
 * It is FREESTANDING (no libc): the research sandbox has no 32-bit libc headers
 * or dynamic loader, so the test talks to the kernel directly via int 0x80.
 * That also makes it runnable under any i686 emulator without a sysroot.
 *
 * Build (both compilers produce a static ELF32):
 *   gcc  -m32 -O2 -nostdlib -ffreestanding -fno-stack-protector -static this.c
 *   lccc -m32 -O2 -nostdlib -ffreestanding -fno-stack-protector this.c
 */

typedef unsigned int u32;
typedef unsigned long long u64;
typedef signed long long i64;

static long sys_write(int fd, const void *buf, unsigned long n) {
    long ret;
    __asm__ volatile("int $0x80"
                     : "=a"(ret)
                     : "a"(4), "b"(fd), "c"(buf), "d"(n)
                     : "memory");
    return ret;
}

static void sys_exit(int code) {
    __asm__ volatile("int $0x80" : : "a"(1), "b"(code));
    __builtin_unreachable();
}

/* Deterministic LCG (no libc rand). */
static u32 rng_state = 0x12345678u;
static u32 nextr(void) {
    rng_state = rng_state * 1664525u + 1013904223u;
    return rng_state;
}

/* ── Select shapes ─────────────────────────────────────────────────────── */
static int maxv(int a, int b) { return a > b ? a : b; }
static int minv(int a, int b) { return a < b ? a : b; }
static int clampv(int x, int lo, int hi) { return x < lo ? lo : (x > hi ? hi : x); }
static int absv(int x) { return x < 0 ? -x : x; }
static int sel_const(int c) { return c ? 42 : -7; }
static int nested(int a, int b, int c) { int m = a > b ? a : b; return m > c ? m : c; }
static int chain(int x) { int r = x; r = r > 100 ? 100 : r; r = r < -100 ? -100 : r; return r; }
static int mix(unsigned u, int s) { return (u & 1) ? s : -s; }
static int eqsel(int a, int b) { return a == b ? a + 1 : b - 1; }
static int unsigned_cmp(unsigned a, unsigned b) { return a > b ? a : b; }

static short maxs(short a, short b) { return a > b ? a : b; }
static short sels(short c) { return c ? (short)1000 : (short)-2000; }
static unsigned short maxus(unsigned short a, unsigned short b) { return a > b ? a : b; }

static unsigned char maxb(unsigned char a, unsigned char b) { return a > b ? a : b; }
static unsigned char selb(unsigned char c) { return c ? (unsigned char)200 : (unsigned char)3; }

/* Select whose arms are themselves expressions (still register-resident). */
static int expr_arms(int a, int b) { return (a & 1) ? (a + b) : (a - b); }

/* Select inside a loop (loop-carried accumulator). */
static int loop_abs_sum(const int *a, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) {
        int v = a[i];
        s += v > 0 ? v : -v;
    }
    return s;
}

/* Select feeding another select (phi chain). */
static int phi_chain(int a, int b, int c, int d) {
    int x = a > b ? a : b;
    int y = c > d ? c : d;
    return x > y ? x : y;
}

/* The kernel hands _start an arbitrarily-aligned stack. The i386 psABI
 * requires 16-byte alignment at call sites, and GCC -O2 emits aligned SSE
 * spills that fault otherwise. This stub aligns %esp before entering the C
 * body, so the corpus runs correctly under BOTH compilers at every -O level
 * without disabling SSE (which would hide stack-alignment bugs). */
__asm__(".text\n"
        ".globl _start\n"
        "_start:\n"
        "    andl $-16, %esp\n"
        "    call test_body\n"
        "    ud2\n");

void test_body(void) {
    u64 acc = 0;
    for (int i = 0; i < 300000; i++) {
        int a = (int)(nextr() >> 1) - 0x20000000;
        int b = (int)(nextr() >> 1) - 0x20000000;
        int c = (int)(nextr() >> 1) - 0x20000000;
        int d = (int)(nextr() >> 1) - 0x20000000;
        acc += (u64)(unsigned)maxv(a, b);
        acc += (u64)(unsigned)minv(a, b);
        acc += (u64)(unsigned)clampv(a, b, c);
        acc += (u64)(unsigned)absv(a);
        acc += (u64)(unsigned)sel_const(a & 1);
        acc += (u64)(unsigned)nested(a, b, c);
        acc += (u64)(unsigned)chain(a);
        acc += (u64)(unsigned)mix((unsigned)a, b);
        acc += (u64)(unsigned)eqsel(a, b);
        acc += (u64)unsigned_cmp((unsigned)a, (unsigned)b);
        acc += (u64)(unsigned)expr_arms(a, b);
        acc += (u64)(unsigned)phi_chain(a, b, c, d);
        short sa = (short)a, sb = (short)b;
        acc += (u64)(unsigned)maxs(sa, sb);
        acc += (u64)(unsigned)sels((short)(a & 1));
        acc += (u64)maxus((unsigned short)a, (unsigned short)b);
        acc += (u64)maxb((unsigned char)a, (unsigned char)b);
        acc += (u64)selb((unsigned char)(a & 1));
    }
    int arr[64];
    for (int i = 0; i < 64; i++)
        arr[i] = (int)(nextr() >> 1) - 0x20000000;
    acc += (u64)(unsigned)loop_abs_sum(arr, 64);

    /* Write the raw 8-byte checksum; the runner compares bytes. */
    unsigned char out[8];
    for (int i = 0; i < 8; i++)
        out[i] = (unsigned char)(acc >> (8 * i));
    sys_write(1, out, 8);
    sys_exit(0);
}
