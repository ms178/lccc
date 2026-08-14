/* Regression test: parameter allocas eliminated in favour of a register
 * "home" must never let one parameter's home destroy another parameter's
 * incoming ABI register.
 *
 * Background. When a parameter's stack slot is never read, the backend can
 * skip the entry-block spill entirely and keep the parameter in the register
 * the allocator gave its ParamRef. That is only sound when the home register
 * is stable, so eligibility is restricted to:
 *   - a callee-saved GPR,
 *   - an XMM register (only assigned to values that do not span calls),
 *   - r11/r10 in a LEAF function.
 *
 * r11 and r10 are the ONLY caller-saved GPRs in the allocation pool that are
 * not also SysV integer argument registers. The rest of that pool is
 * r8/r9/rdi/rsi, which ARE argument registers. Homing a parameter there
 * corrupts a sibling parameter whenever that sibling is materialised in the
 * function BODY instead of by a prologue move:
 *
 *     callee(a,b,c,d,e,f,g,h,i)   homes: a->r11 b->r10 c->r8 d->r9 e->rdi ...
 *     ...
 *     movslq %r8d, %rdi     <- reads e, but c's home already overwrote r8
 *
 * The prologue can order the moves it emits itself (and break cycles with
 * xchg), but it cannot protect reads that happen later in the body, so the
 * ABI-overlapping registers must be rejected at eligibility time.
 *
 * The historical failure that motivated this test: a 9-argument callee
 * returned 17 instead of 21 because arguments 5 and 6 were read after their
 * registers had been reused as homes.
 *
 * Every function below consumes its parameters in a scrambled order so a
 * clobber cannot be masked by a lucky in-order read.
 */

extern int ext_sink(int);
__attribute__((weak)) int ext_sink(int x) { return x & 7; }

/* 9 integer arguments: 6 in registers, 3 on the stack. The historical bug.
 * Arguments are consumed in a permuted order and the stack arguments are
 * checked too, because eliminating the register spill area changes the
 * offsets the stack arguments are read from. */
__attribute__((noinline))
int nine_args(int a, int b, int c, int d, int e, int f, void *g, void *h, void *i)
{
    if (g != 0) return -1;      /* arg7 must be NULL   */
    if (h == 0) return -2;      /* arg8 must be non-NULL */
    if (i != 0) return -3;      /* arg9 must be NULL   */
    /* scrambled consumption order: f, c, a, e, b, d */
    return f * 100000 + c * 10000 + a * 1000 + e * 100 + b * 10 + d;
}

/* Leaf, pointer parameters dereferenced in the body -- the qsort comparator
 * shape that motivated the optimisation. Must stay correct while losing its
 * stack frame. */
__attribute__((noinline))
int cmp_ints(const void *a, const void *b)
{
    return *(const int *)a - *(const int *)b;
}

/* Leaf, six integer parameters read strictly in reverse. */
__attribute__((noinline))
long six_reverse(long p0, long p1, long p2, long p3, long p4, long p5)
{
    long acc = 0;
    acc = acc * 3 + p5;
    acc = acc * 3 + p4;
    acc = acc * 3 + p3;
    acc = acc * 3 + p2;
    acc = acc * 3 + p1;
    acc = acc * 3 + p0;
    return acc;
}

/* Leaf, parameters swapped relative to their ABI registers: a true
 * permutation, the case an unordered prologue would miscompile. */
__attribute__((noinline))
long swapped(long a, long b, long c)
{
    return a * 1 + b * 1000 + c * 1000000;
}

/* NON-leaf: parameters are first read AFTER a call, so their homes must
 * survive the call. r11/r10 are caller-saved, so they are ineligible here
 * and the parameters must land in callee-saved registers or memory. */
__attribute__((noinline))
long live_across_call(long a, long b, long c)
{
    long t = ext_sink(7);
    return a * 100 + b * 10 + c + t;   /* a,b,c read only after the call */
}

/* NON-leaf with a loop: the parameter stays live across many calls. */
__attribute__((noinline))
long live_across_loop(long a, long n)
{
    long s = 0;
    for (long i = 0; i < n; i++)
        s += ext_sink((int)i);
    return s + a;                       /* a live across every iteration */
}

/* Mixed integer/FP: FP homes are XMM, integer homes r11/r10. Both pools are
 * exercised at once, consumed out of order. */
__attribute__((noinline))
double mixed(int a, double x, int b, double y, int c, double z)
{
    return z * 1.0 + (double)c * 2.0 + y * 4.0 + (double)b * 8.0
         + x * 16.0 + (double)a * 32.0;
}

int main(void)
{
    int dummy = 0;

    /* 9 arguments: f=6 c=3 a=1 e=5 b=2 d=4 -> 631524 */
    if (nine_args(1, 2, 3, 4, 5, 6, 0, &dummy, 0) != 631524) return 1;

    {
        static const int arr[2] = { 42, 17 };
        if (cmp_ints(&arr[0], &arr[1]) != 25) return 2;
        if (cmp_ints(&arr[1], &arr[0]) != -25) return 3;
        if (cmp_ints(&arr[0], &arr[0]) != 0) return 4;
    }

    /* ((((5*3+4)*3+3)*3+2)*3+1)*3+0 with acc built from p5 down to p0 */
    if (six_reverse(0, 1, 2, 3, 4, 5) != 1641) return 5;

    if (swapped(7, 8, 9) != 7 + 8000 + 9000000) return 6;

    if (live_across_call(3, 4, 5) != 3 * 100 + 4 * 10 + 5 + (7 & 7)) return 7;

    {
        long want = 0;
        for (long i = 0; i < 5; i++) want += (i & 7);
        if (live_across_loop(6, 5) != want + 6) return 8;
    }

    {
        double got = mixed(1, 2.0, 3, 4.0, 5, 6.0);
        double want = 6.0 + 5 * 2.0 + 4.0 * 4.0 + 3 * 8.0 + 2.0 * 16.0 + 1 * 32.0;
        if (got != want) return 9;
    }

    return 0;
}
