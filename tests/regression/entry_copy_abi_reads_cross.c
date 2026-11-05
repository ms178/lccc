/* Entry-copy coalescing respects the registers the ABI reads and clobbers
 * without naming them.
 *
 * RV64 at -O1 and up: the peephole renames a parameter's register home onto
 * the argument register it arrived in.  Two holes made it miscompile:
 *
 *  - `ret` reads a0/a1 and a call reads a0-a7, yet neither names them.  In
 *    `pass_through` the parameter is homed in s1 (the asm is a call point)
 *    and the body reduces to `mv s1, a0`; coalescing it renamed the epilogue
 *    restore `ld s1, -32(s0)` into `ld a0, -32(s0)`, so the function returned
 *    the caller's s1 (gcc.c-torture/execute/pr40022.c looped forever).
 *  - The indirect call `jalr ra, t2, 0` was not a call to the peephole, so
 *    `add_after_call` had `x` renamed from s1 onto a1 -- which the call
 *    destroys -- and added the callee's leftover a1 instead.
 *
 * `sum_around_calls` pins the back-edge rule: a use above the call inside a
 * loop is reached again after the call.  The cross harness runs the fixture
 * on AArch64 and RV64; the native corpus runs it on x86-64 (and i386).
 */

struct node {
    struct node *next;
};

__attribute__((noinline)) struct node *pass_through(struct node *x)
{
    __asm__ volatile("" : : "g"(x) : "memory");
    return x;
}

__attribute__((noinline)) long pass_through_r(long x)
{
    __asm__ volatile("" : : "r"(x));
    return x;
}

/* Leaves junk in every argument register but the result's. */
__attribute__((noinline)) long clobber_args(void)
{
#if defined(__riscv)
    __asm__ volatile("li a1, 777\n\tli a2, 778\n\tli a3, 779\n\tli a4, 780\n\t"
                     "li a5, 781\n\tli a6, 782\n\tli a7, 783"
                     :
                     :
                     : "a1", "a2", "a3", "a4", "a5", "a6", "a7");
#elif defined(__aarch64__)
    __asm__ volatile("mov x1, #777\n\tmov x2, #778\n\tmov x3, #779\n\tmov x4, #780\n\t"
                     "mov x5, #781\n\tmov x6, #782\n\tmov x7, #783"
                     :
                     :
                     : "x1", "x2", "x3", "x4", "x5", "x6", "x7");
#elif defined(__x86_64__)
    __asm__ volatile("movl $777, %%esi\n\tmovl $778, %%edx\n\tmovl $779, %%ecx\n\t"
                     "movl $780, %%r8d\n\tmovl $781, %%r9d"
                     :
                     :
                     : "rsi", "rdx", "rcx", "r8", "r9");
#endif
    return 1000;
}

__attribute__((noinline)) long twice_clobbering(long v)
{
    (void)clobber_args();
    return v * 2;
}

__attribute__((noinline)) long add_after_call(long (*fp)(void), long x)
{
    long r = fp();
    return r + x;
}

__attribute__((noinline)) long mul_add_after_call(long (*fp)(long), long a, long b, long c)
{
    long r = fp(a);
    return r * b + c;
}

__attribute__((noinline)) long sum_around_calls(long dummy, long x)
{
    long s = dummy;
    for (int i = 0; i < 3; i++) {
        s += x;
        (void)clobber_args();
    }
    return s;
}

/* Keeps values live in callee-saved registers across the calls, so a callee
 * that hands back one of the caller's registers instead of its result is
 * caught by value. */
__attribute__((noinline)) int run(long seed)
{
    static struct node e;
    long k0 = seed * 3 + 1, k1 = seed * 5 + 2, k2 = seed * 7 + 3, k3 = seed * 11 + 4;
    int bad = 0;
    if (pass_through(&e) != &e)
        bad |= 1;
    if (pass_through_r(k1 ^ 0x5a5a) != (k1 ^ 0x5a5a))
        bad |= 2;
    if (add_after_call(clobber_args, k0) != 1000 + k0)
        bad |= 4;
    if (mul_add_after_call(twice_clobbering, k1, k2, k3) != 2 * k1 * k2 + k3)
        bad |= 8;
    if (sum_around_calls(k2, k3) != k2 + 3 * k3)
        bad |= 16;
    return bad | (k0 + k1 + k2 + k3 != seed * 26 + 10 ? 32 : 0);
}

int main(void)
{
    return run(41);
}
