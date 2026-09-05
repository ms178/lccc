/*
 * `-std=` must select `__STDC_VERSION__`, and C23's one-argument `va_start`
 * must work as a consequence.
 *
 * `__STDC_VERSION__` was pinned at the C17 default (`201710L`) no matter what
 * `-std=` said. Every C23-conditional header therefore took its pre-C23
 * branch even under `-std=c23`. The visible casualty was GCC's own
 * <stdarg.h>:
 *
 *     #if __STDC_VERSION__ > 201710L
 *     # define va_start(v, ...) __builtin_va_start(v, 0)
 *     #else
 *     # define va_start(v, l)   __builtin_va_start(v, l)
 *     #endif
 *
 * so the C23 form `va_start(ap)` expanded through the two-parameter branch to
 * `__builtin_va_start(ap,)` and died in the parser with "expected expression
 * before ')'" — gcc.c-torture/execute/pr117432.c, which the host GCC 14.2
 * cannot compile at all.
 *
 * This test is compiled with `-std=c23` (see the .flags sidecar) and pins:
 *   1. `__STDC_VERSION__ == 202311L` under that dialect;
 *   2. C23 unnamed variadic parameters `void f(...)`;
 *   3. the one-argument `va_start(ap)` reaching the correct first variadic
 *      argument, for several argument types and mixed promotions;
 *   4. `va_copy` / `va_end` over the same list;
 *   5. a second traversal after `va_end`, so a mis-initialised `ap` shows up
 *      as a wrong value rather than accidentally-right garbage.
 */
#include <stdarg.h>
#include <stdio.h>

#if !defined(__STDC_VERSION__)
#error "__STDC_VERSION__ must be defined under -std=c23"
#endif
#if __STDC_VERSION__ < 202311L
#error "-std=c23 must select __STDC_VERSION__ >= 202311L"
#endif

static int fails;

#define CHECK(cond, ...)                                                                 \
    do {                                                                                 \
        if (!(cond)) {                                                                   \
            printf(__VA_ARGS__);                                                         \
            ++fails;                                                                     \
        }                                                                                \
    } while (0)

/* C23: a variadic function with NO named parameters. */
__attribute__((noinline)) static long long sum_ll(...) {
    va_list ap;
    va_start(ap); /* one-argument form */
    long long n = va_arg(ap, int);
    long long acc = 0;
    for (long long i = 0; i < n; i++)
        acc += va_arg(ap, long long);
    va_end(ap);
    return acc;
}

/* Mixed widths + a double, to exercise the SysV register/overflow split. */
__attribute__((noinline)) static double mixed(...) {
    va_list ap, cp;
    va_start(ap);
    va_copy(cp, ap);

    int a = va_arg(ap, int);
    double b = va_arg(ap, double);
    long long c = va_arg(ap, long long);
    unsigned d = va_arg(ap, unsigned);
    va_end(ap);

    /* Second, independent traversal of the same list. */
    int a2 = va_arg(cp, int);
    double b2 = va_arg(cp, double);
    va_end(cp);

    if (a != a2 || b != b2)
        return -1.0;
    return (double) a + b + (double) c + (double) d;
}

/* Enough arguments to spill past the six SysV integer registers. */
__attribute__((noinline)) static long overflow_args(...) {
    va_list ap;
    va_start(ap);
    long acc = 0;
    for (int i = 0; i < 12; i++)
        acc = acc * 3 + va_arg(ap, int);
    va_end(ap);
    return acc;
}

int main(void) {
    printf("__STDC_VERSION__ = %ldL\n", (long) __STDC_VERSION__);

    CHECK(sum_ll(3, 10LL, 20LL, 30LL) == 60, "sum_ll = %lld want 60\n", sum_ll(3, 10LL, 20LL, 30LL));
    CHECK(sum_ll(0) == 0, "sum_ll(0) != 0\n");
    CHECK(sum_ll(1, -5LL) == -5, "sum_ll(1,-5) wrong\n");

    {
        double got = mixed(7, 0.5, 100LL, 3u);
        CHECK(got == 110.5, "mixed = %g want 110.5\n", got);
    }

    {
        long want = 0;
        for (int i = 0; i < 12; i++)
            want = want * 3 + (i + 1);
        long got = overflow_args(1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12);
        CHECK(got == want, "overflow_args = %ld want %ld\n", got, want);
    }

    printf("c23_stdc_version_va_start: %s\n", fails ? "FAIL" : "OK");
    return fails != 0;
}
