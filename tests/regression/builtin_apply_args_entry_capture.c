/* __builtin_apply_args() describes the call the CURRENT FUNCTION received, so
   the argument registers have to be captured where they still hold it: at
   entry.  On x86-64 the builtin used to read them where it stood in the body.
   Pinned here, all older than this file:

   live     the save staged its block pointer in %r10, a register the
            allocator hands out.  `sum8` and `mixed6` returned garbage at -O1
            and above; `wide` joins them once the parameters of a 128-bit body
            may live in %r10/%r11.
   late     anything that ran before the builtin had already overwritten the
            argument registers (`forward_late`).
   vector   the %al test in front of the %xmm0-%xmm7 save, and the one in
            front of their reload in __builtin_apply, jumped unconditionally:
            no vector argument was ever saved or restored (`forward_fp`
            returned 35.5 for 32 at -O0).
   %al      was read after the block size had been loaded into %eax, so a
            forwarded variadic call told its callee a random register count
            (`forward_variadic`).

   Without SSE (the _nosse variant) only the integer half exists. */
#include <stdarg.h>
#include <stdio.h>

#define NOINLINE __attribute__((noinline, noclone))

static void *volatile sink;
static int failures;

static void expect(const char *what, long got, long want) {
    if (got != want) {
        printf("%s: got %ld, want %ld\n", what, got, want);
        failures++;
    }
}

/* Eight parameters live across the builtin: more than the registers no
   argument arrives in. */
NOINLINE static long sum8(long a, long b, long c, long d, long e, long f, long g, long h) {
    sink = __builtin_apply_args();
    return a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h;
}

/* A temporary born before the builtin as well. */
NOINLINE static long mixed6(long a, long b, long c, long d, long e, long f) {
    long t = a * b;
    sink = __builtin_apply_args();
    return t + a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f;
}

#ifdef __SIZEOF_INT128__
NOINLINE static unsigned __int128 wide(unsigned long a, unsigned long b) {
    sink = __builtin_apply_args();
    return (unsigned __int128)a * b;
}
#endif

NOINLINE static long target_int(long a, long b, long c, long d, long e, long f) {
    return a + 10 * b + 100 * c + 1000 * d + 10000 * e + 100000 * f;
}

NOINLINE static long forward_int(long a, long b, long c, long d, long e, long f) {
    __builtin_return(__builtin_apply((void (*)())target_int, __builtin_apply_args(), 64));
}

/* Every argument register is rewritten before the builtin runs. */
NOINLINE static long scramble(long a, long b, long c, long d, long e, long f) {
    return (a ^ b) + (c ^ d) + (e ^ f);
}

NOINLINE static long forward_late(long a, long b, long c, long d, long e, long f) {
    sink = (void *)scramble(-1, -2, -3, -4, -5, -6);
    __builtin_return(__builtin_apply((void (*)())target_int, __builtin_apply_args(), 64));
}

NOINLINE static long target_sum(int n, ...) {
    va_list ap;
    long s = 0;
    va_start(ap, n);
    for (int i = 0; i < n; i++)
        s = s * 7 + va_arg(ap, long);
    va_end(ap);
    return s;
}

NOINLINE static long forward_sum(int n, ...) {
    __builtin_return(__builtin_apply((void (*)())target_sum, __builtin_apply_args(), 64));
}

#ifdef __SSE2__
NOINLINE static double target_fp(int a, double x, long b, double y) { return a + x * 2 + b * 3 + y * 4; }

NOINLINE static double forward_fp(int a, double x, long b, double y) {
    void *args = __builtin_apply_args();
    __builtin_return(__builtin_apply((void (*)())target_fp, args, 64));
}

NOINLINE static double scramble_fp(double a, double b, double c, double d) { return a * b + c * d; }
static volatile double fp_sink;

NOINLINE static double forward_fp_late(int a, double x, long b, double y) {
    fp_sink = scramble_fp(9.0, 8.0, 7.0, 6.0);
    __builtin_return(__builtin_apply((void (*)())target_fp, __builtin_apply_args(), 64));
}

NOINLINE static double target_variadic(int n, ...) {
    va_list ap;
    double s = 0;
    va_start(ap, n);
    for (int i = 0; i < n; i++)
        s += va_arg(ap, double) * (i + 1);
    va_end(ap);
    return s;
}

NOINLINE static double forward_variadic(int n, ...) {
    __builtin_return(__builtin_apply((void (*)())target_variadic, __builtin_apply_args(), 64));
}
#endif

int main(void) {
    expect("sum8", sum8(1, 2, 3, 4, 5, 6, 7, 8), 204);
    expect("mixed6", mixed6(1, 2, 3, 4, 5, 6), 93);
#ifdef __SIZEOF_INT128__
    unsigned __int128 w = wide(0xfffffffffffffffful, 0xfffffffffffffffeul);
    expect("wide.hi", (long)(unsigned long)(w >> 64), (long)0xfffffffffffffffdul);
    expect("wide.lo", (long)(unsigned long)w, 2);
#endif
    expect("forward_int", forward_int(1, 2, 3, 4, 5, 6), 654321);
    expect("forward_late", forward_late(6, 5, 4, 3, 2, 1), 123456);
    expect("forward_sum", forward_sum(5, 1L, 2L, 3L, 4L, 5L), 3267);
#ifdef __SSE2__
    expect("forward_fp", (long)(forward_fp(1, 2.5, 3, 4.25) * 100), 3200);
    expect("forward_fp_late", (long)(forward_fp_late(2, 0.5, 4, 0.25) * 100), 1600);
    expect("forward_variadic", (long)(forward_variadic(3, 1.5, 2.5, 3.5) * 100), 1700);
    expect("forward_variadic.8", (long)forward_variadic(8, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0), 204);
#endif
    printf("%s\n", failures ? "FAIL" : "ok");
    return failures != 0;
}
