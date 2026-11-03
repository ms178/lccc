/* Writing a trampoline must not clobber a value that is live across it.
 *
 * gcc.c-torture/execute/nestfunc-3.c: `nested_1(a, b)` takes the address of a
 * sibling nested function -- which writes a trampoline into its frame -- and
 * then passes `b` on to a call:
 *
 *     long nested_1 (long a, long b) { return use (nested_0, b, a) + sum; }
 *
 * At -O1 and above LCCC kept `b` in its incoming %rsi, the x86-64 trampoline
 * writer staged the static chain in %rsi, and `use` received a stack address
 * instead of `b` (the torture test aborted at -O1, -O2, -O3 and -Os).  The
 * writer stages the buffer, the chain and the target in argument registers on
 * every backend, so `InitTrampoline` is now a call point for register
 * allocation, like the inline `Memcpy` expansion.
 *
 * Each function below keeps a different pair of incoming argument registers
 * live across one trampoline initialisation and consumes them only in the
 * call that follows (so no real call in between forces them out of the
 * argument registers first); `two_trampolines` writes two in a row.  GCC is
 * the differential oracle.
 */
#include <stdio.h>

typedef long (*fn2)(long, long);

__attribute__((noinline)) long apply2(fn2 f, long x, long y) { return f(x, y); }
__attribute__((noinline)) long apply2_swapped(fn2 f, long x, long y) { return f(y, x); }

/* %rdi and %rsi (x0/x1, a0/a1) live across the initialisation. */
__attribute__((noinline)) long pass_first_pair(long a, long b) {
    long k = 3;
    long g(long x, long y) { return x * 2 - y + k; }
    return apply2(g, a, b);
}

/* %rdx and %rcx: the x86-64 writer loads the target address into %rcx. */
__attribute__((noinline)) long pass_second_pair(long a, long b, long c, long d) {
    long k = 5 + a - b;
    long g(long x, long y) { return x - 3 * y + k; }
    return apply2(g, c, d);
}

/* %r8 and %r9 (x4/x5, a4/a5). */
__attribute__((noinline)) long pass_third_pair(long a, long b, long c, long d, long e, long f) {
    long k = 7 + a + b + c + d;
    long g(long x, long y) { return x * y + k; }
    return apply2(g, e, f);
}

/* Two trampolines back to back, both arguments forwarded after each. */
__attribute__((noinline)) long two_trampolines(long a, long b) {
    long k = a - b;
    long f1(long x, long y) { return x - y + k; }
    long f2(long x, long y) { return x * y - k; }
    long r = apply2(f1, a, b);
    return r + apply2_swapped(f2, b, a);
}

int main(void) {
    /* The torture test's own shape. */
    long sum = 0;
    long nested_0(long a, long b) { return a > 2 * b ? a - b : b - a; }
    long nested_1(long a, long b) { return apply2_swapped(nested_0, b, a) + sum; }
    long nested_2(long a, long b) { return nested_1(b, a); }
    for (long i = 0; i < 6; ++i)
        for (long j = 0; j < 6; ++j)
            sum += apply2(nested_2, i, j > 2 ? j - 2 : 2 - j);
    printf("nestfunc-3 shape: %ld\n", sum);

    static const long v[] = {0, 1, -1, 7, -9, 1L << 40, -(1L << 33)};
    unsigned long mix = 1469598103934665603UL;
    for (unsigned i = 0; i < sizeof v / sizeof v[0]; ++i)
        for (unsigned j = 0; j < sizeof v / sizeof v[0]; ++j) {
            long a = v[i], b = v[j];
            long r[4] = {pass_first_pair(a, b), pass_second_pair(b, a, a, b),
                         pass_third_pair(a, b, 2, 3, b, a), two_trampolines(a, b)};
            for (unsigned n = 0; n < 4; ++n)
                mix = (mix ^ (unsigned long)r[n]) * 1099511628211UL;
        }
    printf("pass_first_pair(9, 4) = %ld\n", pass_first_pair(9, 4));
    printf("pass_second_pair(1, 2, 30, 4) = %ld\n", pass_second_pair(1, 2, 30, 4));
    printf("pass_third_pair(1, 2, 3, 4, 50, 6) = %ld\n", pass_third_pair(1, 2, 3, 4, 50, 6));
    printf("two_trampolines(9, 4) = %ld\n", two_trampolines(9, 4));
    printf("mix = %016lx\n", mix);
    return 0;
}
