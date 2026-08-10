/* v4 regression: integer arithmetic correctness — signedness, promotion,
 * overflow, division, modulo. Verifies against compile-time-known values. */
#include <stdio.h>
#include <limits.h>

int main(void) {
    /* signed/unsigned promotion */
    unsigned u = 1;
    int i = -2;
    if ((int)(u + i) != -1) return 1;          /* u+i promotes to unsigned: 1u + (unsigned)-2 = 0xffffffff */
    if ((u > i) != 0) return 2;                 /* -2 converts to huge unsigned: 1 > 0xFFFFFFFE = false */
    if ((signed)(u > i ? 1 : 0) != 0) return 3;

    /* division/modulo sign rules */
    if (-7 / 2 != -3) return 4;
    if (-7 % 2 != -1) return 5;
    if (7 / -2 != -3) return 6;
    if (7 % -2 != 1) return 7;
    if (-7u / 2u != (unsigned)(-7) / 2u) return 8;

    /* overflow wraps (2's complement, defined in C23/GNU) */
    if ((int)(INT_MAX + 1) != INT_MIN) return 9;
    if ((unsigned)(-1) + 1u != 0u) return 10;

    /* 64-bit arithmetic */
    long long a = 0x123456789abcdef0LL;
    long long b = 0xfedcba9876543210LL;
    if (a * b != (long long)(0x123456789abcdef0LL * 0xfedcba9876543210LL)) return 11;
    if ((a >> 4) != 0x0123456789abcdefLL) return 12;  /* arithmetic shift */
    if ((a << 8) != (long long)(0x3456789abcdef000LL)) return 13;
    unsigned long long ua = 0xf000000000000000ULL;
    if ((ua >> 60) != 0xfULL) return 14;              /* logical shift */

    /* shifts with variable counts */
    int n = 5;
    if ((1 << n) != 32) return 15;
    if ((0x80000000u >> n) != 0x04000000u) return 16;

    /* mixed-width */
    short s = -1000;
    int si = s * 3;
    if (si != -3000) return 17;
    char c = 100;
    if (c + 100 != 200) return 18;

    printf("OK int_arith\n");
    return 0;
}
