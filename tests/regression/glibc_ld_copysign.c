/* glibc_ld_copysign.c — long-double copysign/fabs via the pure-GPR bit-op
 * implementation (no x87 round-trip). glibc k_casinhl.c used to panic
 * "LDCopysign: unsupported operand shape". Values flow through functions
 * (parameter -> bit-op -> return), the shape libm actually uses, and are
 * compared with IEEE extended semantics (x87 fcomi). */
#include <stdio.h>

static long double __attribute__((noinline)) cs(long double x, long double y) {
    return __builtin_copysignl(x, y);
}
static long double __attribute__((noinline)) fa(long double x) {
    return __builtin_fabsl(x);
}

int main(void) {
    long double a = cs(3.0L, -1.0L);      /* -3.0 */
    long double b = cs(-2.0L, 3.0L);      /* +2.0 */
    long double c = fa(-4.5L);             /* +4.5 */
    long double d = cs(3.0L, -2.0L);       /* -3.0 (const sign) */
    if (!(a == -3.0L)) { printf("FAIL ld copysign\n"); return 1; }
    if (!(b == 2.0L)) { printf("FAIL ld copysign 2\n"); return 1; }
    if (!(c == 4.5L)) { printf("FAIL ld fabs\n"); return 1; }
    if (!(d == -3.0L)) { printf("FAIL ld copysign 3\n"); return 1; }
    printf("PASS ld_copysign\n");
    return 0;
}
