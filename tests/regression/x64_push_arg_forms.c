/* x86-64: outgoing MEMORY-class scalar stack arguments are staged with the
 * direct push forms — `pushq $imm`, `pushq %reg`, `pushq slot(%rsp)` — and
 * the parity pad below the outgoing area is `pushq $0`, not `subq $8,%rsp`.
 *
 * Encoding law being pinned (each strictly beats materialise-into-%rax +
 * `pushq %rax`):
 *
 *     imm in [-128,127]   pushq $imm8    2 B   (fallback: 6 B)
 *     imm in imm32        pushq $imm32   5 B   (fallback: 6 B)
 *     zero                pushq $0       2 B   (fallback: 3 B, xorl+push)
 *     register home       pushq %reg     1-2 B (fallback: 5-6 B)
 *     scalar slot         pushq mem      5-8 B (fallback: 6-9 B, + %rax chain)
 *
 * Three things can go wrong, and all are observable here as wrong values:
 *
 *   * SIGN EXTENSION: `pushq $imm32` writes all eight slot bytes as
 *     sign_extend(imm).  For eight-byte arguments the value must fit the
 *     imm32 window exactly (0x8000_0000 as an unsigned 64-bit value must
 *     NOT be pushed as an imm32 — it would arrive sign-extended); for
 *     arguments narrower than eight bytes the upper lane is unspecified by
 *     the SysV AMD64 ABI and the callee reads only the low lane;
 *   * SLOT STALENESS: a slot push reads the authoritative copy — the same
 *     location the fallback loads — while the frame accounting absorbs
 *     every earlier push of the marshalling sequence; a miscount shifts a
 *     later slot push (or a register-argument load) into the wrong slot;
 *   * HOME FRESHNESS: a register push must read a value the register still
 *     holds; pushing a stale home publishes a derived value.
 *
 * The caller stages every shape in every stack position: constants at the
 * imm8 boundaries (127/-128/128/-129), zero as int and as double 0.0, an
 * unsigned value outside the imm32 window (must take the movabsq fallback,
 * NOT sign-extend), floats, globals and locals across the window,
 * sub-int types, with SSE register arguments in front (SSE staging and GP
 * push staging share one marshalling window) and a variadic tail (the al
 * census stays live across GP pushes).  The callees read their stack
 * parameters repeatedly and mutate one through its address, so a mis-staged
 * slot shows up downstream of the call, not just at it.
 *
 * Reference values are plain C semantics (gcc/clang -O0..-O3 agree; the
 * expected constants below are derived by hand from that semantics). */
#include <stdio.h>

long g_sink;

/* Fifteen parameters: six in registers, nine on the stack — several pushes
 * inside one marshalling sequence, and a mutation through the address of a
 * stack parameter so the callee's incoming slots must be real. */
static long take9(long a, long b, long c, long d, long e, long f, long s1,
                  long s2, long s3, long s4, long s5, long s6, long s7,
                  long s8, long s9)
{
    long *p = &s5;
    *p += s1;
    g_sink += s9;
    return s1 + 2 * s2 + 3 * s3 + 4 * s4 + 5 * s5 + 6 * s6 + 7 * s7 + 8 * s8
           + 9 * s9 + a + b + c + d + e + f;
}

static long varargs_sum(int n, ...)
{
    __builtin_va_list ap;
    __builtin_va_start(ap, n);
    long sum = 0;
    for (int i = 0; i < n; i++)
        sum += __builtin_va_arg(ap, long);
    __builtin_va_end(ap);
    return sum;
}

/* A double register argument in front, double AND long stack scalars
 * behind: one marshalling window, two register classes. */
static double mixed(double x, long a, long b, long c, long d, long e, long f,
                    double s1, double s2, long s3)
{
    return x + (double)(a + b + c + d + e + f) + s1 + s2 + (double)s3;
}

/* Indirect call: the target is spilled before the pushes move %rsp; the
 * reload must not read a shifted slot. */
static long (*volatile fp9)(long, long, long, long, long, long, long, long,
                            long, long, long, long, long, long, long);
static long ident9(long a, long b, long c, long d, long e, long f, long s1,
                   long s2, long s3, long s4, long s5, long s6, long s7,
                   long s8, long s9)
{
    return s1 - s2 + s3 - s4 + s5 - s6 + s7 - s8 + s9 + a + e;
}

/* Sub-int stack parameters (mixed with longs at the ABI's eightbyte
 * stride): the pushed low lanes must be exact, the ignored upper lanes
 * must not leak into any read. */
static unsigned long narrow_args(int a, short b, char c, unsigned int d,
                                 long e, long f, int s1, short s2, char s3,
                                 unsigned u1, long s5)
{
    return (unsigned long)(a + b + c + (int)d + s1 + s2 + s3 + (int)u1 + s5
                           + (int)(e - f));
}

int main(void)
{
    /* --- imm8 window boundaries, zero, imm32 range -------------------- */
    long r1 = take9(1, 2, 3, 4, 5, 6, 127, -128, 0, 128, -129, 0x7fffffff,
                    -1, 1, -2);
    /* s5 mutates: -129 + 127 = -2.  Return:
     * 127 - 256 + 0 + 512 - 10 + 6*2147483647 - 7 + 8 - 18 + 21
     *   = 12884902259; g_sink = -2. */
    printf("take9: %ld (expect 12884902259)\n", r1);
    if (r1 != 12884902259L || g_sink != -2)
        return 1;

    /* --- unsigned 64-bit outside the imm32 window --------------------- */
    unsigned long big = 0x80000000UL;
    long r2 = take9(0, 0, 0, 0, 0, 0, big, 44, 45, 46, 47, 48, 49, 50, 51);
    /* s5 mutates: 47 + 0x80000000 = 2147483695 (needs the FULL 64-bit
     * value; a sign-extending push of the low imm32 would deliver
     * 0xffffffff8000002f here).  Return:
     * 0x80000000 + 88 + 135 + 184 + 5*2147483695 + 288 + 343 + 400 + 459
     *   = 12884904020; g_sink = -2 + 51 = 49. */
    printf("take9-big: %ld (expect 12884904020)\n", r2);
    if (r2 != 12884904020L || g_sink != 49)
        return 2;

    /* --- locals and globals across the marshalling window ------------- */
    long loc = 1000;
    long r3 = take9(loc, -loc, 7, 4, 5, 6, loc + 1, loc + 2, loc + 3,
                    loc + 4, loc + 5, loc + 6, loc + 7, loc + 8, loc + 9);
    /* s5 mutates: 1005 + 1001 = 2006.  Return:
     * 1001 + 2004 + 3009 + 4016 + 10030 + 6036 + 7049 + 8064 + 9081 + 22
     *   = 50312; g_sink = 49 + 1009 = 1058. */
    printf("take9-loc: %ld (expect 50312)\n", r3);
    if (r3 != 50312L || g_sink != 1058)
        return 3;

    /* --- variadic tail after GP pushes -------------------------------- */
    long r4 = varargs_sum(3, 1, 2, 3);
    long r5 = varargs_sum(9, 1, 2, 3, 4, 5, 6, 7, 8, 9);
    printf("va: %ld,%ld (expect 6,45)\n", r4, r5);
    if (r4 != 6 || r5 != 45)
        return 4;

    /* --- mixed SSE register + GP stack staging ------------------------ */
    double r6 = mixed(1.5, 1, 2, 3, 4, 5, 6, 0.25, -0.5, 9);
    printf("mixed: %.4g (expect 31.25)\n", r6);
    if (r6 != 31.25)
        return 5;

    /* --- indirect call: fptr spill across the pushes ------------------ */
    fp9 = ident9;
    long r7 = fp9(1, 2, 3, 4, 5, 6, 10, 20, 30, 40, 50, 60, 70, 80, 90);
    /* 10-20+30-40+50-60+70-80+90 + 1 + 5 = 56. */
    printf("indirect: %ld (expect 56)\n", r7);
    if (r7 != 56)
        return 6;

    /* --- narrow stack scalars: exact low lanes ------------------------ */
    unsigned long r8 = narrow_args(1, 2, 3, 4u, 5, 6, 7, 8, 9, 10u, 11);
    /* 1+2+3+4+7+8+9+10+11 + (5-6) = 54. */
    printf("narrow: %lu (expect 54)\n", r8);
    if (r8 != 54)
        return 7;

    /* --- double 0.0 as a stack scalar pushes as $0 -------------------- */
    double zero = 0.0;
    double r9 = mixed(0.0, 0, 0, 0, 0, 0, 0, zero, zero, (long)zero);
    printf("zeros: %.4g (expect 0)\n", r9);
    if (r9 != 0.0)
        return 8;

    return 0;
}
