/* A label reached both by an asm goto and by falling through from its block.
 *
 * An asm goto leaves its block from where it stands, and what stands behind
 * it in the block has not run on that way out.  Two parts of the compiler
 * treated the block as leaving from its end only:
 *
 * The promotion of locals to SSA placed phis block by block, so a variable
 * assigned behind the asm goto had one value at the label for both ways in --
 * the one from before the assignment:
 *
 *     int err = -14;
 *     asm goto("..." : : : : out);    // unsafe_put_user(), a static branch
 *     err = 0;
 *   out:
 *     return err;                     // -14 after falling through, too
 *
 * at every optimization level, -O0 included (GCC 16.2 and Clang 23.1.5: 0).
 *
 * Leaving SSA put the copies into a label's merged variables at the end of a
 * block whose only successor is that label, asm goto or not: `merged` below
 * reached `out` by the asm goto with whatever the three registers held
 * (-O1 and above: 20 or 191 for 87).
 *
 * The shapes differ in what stands between the asm goto and the label: a
 * plain store, a call's result, a loop that falls out at the label, a second
 * store ahead of the asm goto, two asm gotos to one label, a variable wider
 * than `int` beside a pointer, and calls only.  `later_block` is the control:
 * its stores sit in blocks of their own and were always right.
 *
 * Deliberately header-free, like asm_goto_phi_edge.c: it is compiled for
 * -m32 as well, and nothing here needs a libc header.
 */
extern int printf(const char *, ...);

#if defined(__x86_64__) || defined(__i386__)
#define JUMP_IF(x, label) __asm__ goto("testl %0, %0; jnz %l[" #label "]" : : "r"(x) : "cc" : label)
#elif defined(__aarch64__)
#define JUMP_IF(x, label) __asm__ goto("cbnz %w0, %l[" #label "]" : : "r"(x) : : label)
#elif defined(__riscv)
#define JUMP_IF(x, label) __asm__ goto("bnez %0, %l[" #label "]" : : "r"(x) : : label)
#else
#error unsupported target
#endif

#define NOINLINE __attribute__((noinline))
static volatile int zero;

NOINLINE static int same_block(int x)
{
    int err = -14;
    JUMP_IF(x, out);
    err = 0;
out:
    return err;
}

NOINLINE static int twice(int v)
{
    return 2 * v + zero;
}

NOINLINE static int after_call(int x, int v)
{
    int r = -1;
    JUMP_IF(x, out);
    r = twice(v);
out:
    return r;
}

NOINLINE static int loop_exit(int x, int trips)
{
    int s = 0, j = 0;
    do {
        JUMP_IF(x, out);
        s += 10 + j;
    } while (++j < trips);
out:
    return s;
}

NOINLINE static int between(int x)
{
    int r = 1;
    r += 10;
    JUMP_IF(x, out);
    r += 100;
out:
    return r;
}

NOINLINE static int later_block(int x, int y)
{
    int r = 1;
    JUMP_IF(x, out);
    if (y)
        r = 2;
    else
        r = 3;
out:
    return r;
}

NOINLINE static int two_gotos(int x, int y)
{
    int r = 1;
    JUMP_IF(x, out);
    r = 2;
    JUMP_IF(y, out);
    r = 3;
out:
    return r;
}

NOINLINE static long long wide(int x, long long v)
{
    long long r = -1;
    const char *p = "ab";
    JUMP_IF(x, out);
    r = v * 3;
    p++;
out:
    return r + *p;
}

NOINLINE static void ext(void)
{
    zero += 0;
}

/* Nothing is stored behind this asm goto.  The label merges three values
 * with another path, and the asm goto's block brings the same three both
 * ways: the copies into the merged variables belong on each. */
NOINLINE static long long merged(int x, int y, long long a, long long b, long long c, long long d)
{
    long long r = a * 3 + b, q = c * 5 + d, t = a ^ d;
    if (y) {
        r = b + 1;
        q = c + 2;
        t = 7;
        goto out;
    }
    JUMP_IF(x, out);
    ext();
    ext();
    ext();
out:
    return r + q * 3 + t * 5;
}

int main(void)
{
    int a0 = same_block(zero), a1 = same_block(1 + zero);
    int b0 = after_call(zero, 21), b1 = after_call(1 + zero, 21);
    int c0 = loop_exit(zero, 3), c1 = loop_exit(1 + zero, 3);
    int d0 = between(zero), d1 = between(1 + zero);
    int e0 = later_block(zero, 1), e1 = later_block(zero, 0), e2 = later_block(1 + zero, 1);
    int f0 = two_gotos(zero, zero), f1 = two_gotos(zero, 1 + zero), f2 = two_gotos(1 + zero, zero);
    long long g0 = wide(zero, 5), g1 = wide(1 + zero, 5);
    long long h0 = merged(zero, zero, 1, 2, 3, 4), h1 = merged(1 + zero, zero, 1, 2, 3, 4);
    long long h2 = merged(zero, 1 + zero, 1, 2, 3, 4);

    printf("same_block %d %d\n", a0, a1);
    printf("after_call %d %d\n", b0, b1);
    printf("loop_exit %d %d\n", c0, c1);
    printf("between %d %d\n", d0, d1);
    printf("later_block %d %d %d\n", e0, e1, e2);
    printf("two_gotos %d %d %d\n", f0, f1, f2);
    printf("wide %lld %lld\n", g0, g1);
    printf("merged %lld %lld %lld\n", h0, h1, h2);

    return !(a0 == 0 && a1 == -14 && b0 == 42 && b1 == -1 && c0 == 33 && c1 == 0 &&
             d0 == 111 && d1 == 11 && e0 == 2 && e1 == 3 && e2 == 1 &&
             f0 == 3 && f1 == 2 && f2 == 1 && g0 == 15 + 'b' && g1 == -1 + 'a' &&
             h0 == 87 && h1 == 87 && h2 == 53);
}
