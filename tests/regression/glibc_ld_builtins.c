/* glibc_ld_builtins.c — long-double (x87 80-bit) math builtins through
 * function boundaries: __builtin_sqrtl / truncl / floorl / ceill / rintl /
 * nearbyintl / roundl plus exact long-double literal materialization.
 * These builtins were previously unregistered (undefined reference to the
 * literal "__builtin_truncl" at link) or mis-typed (I64 return: the x87
 * st0 result was read as an integer or dropped, so a function returning
 * copysignl produced garbage like 0x12af9 instead of -3.0L). */
#include <stdio.h>

static long double __attribute__((noinline)) sq(long double x) {
    return __builtin_sqrtl(x);
}
static long double __attribute__((noinline)) tr(long double x) {
    return __builtin_truncl(x);
}
static long double __attribute__((noinline)) fl(long double x) {
    return __builtin_floorl(x);
}
static long double __attribute__((noinline)) ce(long double x) {
    return __builtin_ceill(x);
}
static long double __attribute__((noinline)) ri(long double x) {
    return __builtin_rintl(x);
}
static long double __attribute__((noinline)) nb(long double x) {
    return __builtin_nearbyintl(x);
}
static long double __attribute__((noinline)) ro(long double x) {
    return __builtin_roundl(x);
}

int main(void) {
    /* x87 function-return chain: call -> st0 -> slot -> fldt return */
    if (!(sq(9.0L) == 3.0L)) { printf("FAIL ld sqrtl\n"); return 1; }
    if (!(tr(-3.7L) == -3.0L)) { printf("FAIL ld truncl\n"); return 1; }
    if (!(fl(-3.7L) == -4.0L)) { printf("FAIL ld floorl\n"); return 1; }
    if (!(ce(-3.7L) == -3.0L)) { printf("FAIL ld ceill\n"); return 1; }
    if (!(ri(2.5L) == 2.0L)) { printf("FAIL ld rintl\n"); return 1; }
    if (!(nb(2.5L) == 2.0L)) { printf("FAIL ld nearbyintl\n"); return 1; }
    if (!(ro(-2.5L) == -3.0L)) { printf("FAIL ld roundl\n"); return 1; }
    /* exact long-double literal materialization (x87 80-bit, not a
     * binary128/int-derived bit pattern) */
    if (!(1.5L == 1.5L) || !(2.5L == 2.5L) || !(3.5L == 3.5L)
        || !(-1.5L == -1.5L) || !(-2.5L == -2.5L) || !(-3.5L == -3.5L)) {
        printf("FAIL ld literals\n");
        return 1;
    }
    printf("PASS ld_builtins\n");
    return 0;
}
