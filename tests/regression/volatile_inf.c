/* Regression (v5, BUG-005 checks): volatile FP stores of +Inf must survive,
 * and frexp(+Inf) with a volatile source must return the identical value.
 *
 * 1) `volatile double x = 1.0/0.0` — the constant-folded division must not
 *    collapse the volatile store (bit pattern 0x7ff0000000000000).
 * 2) frexp(volatile +Inf) — the exponent must be 0 and the fraction must
 *    compare equal to the input (both +Inf), regardless of the volatile
 *    reload path. */
#include <math.h>
#include <stdio.h>
#include <string.h>

static int check_volatile_inf_store(void) {
    volatile double x = 1.0;
    double zero = 0.0;
    x = 1.0 / zero;
    double c = x;
    unsigned long long b;
    memcpy(&b, &c, 8);
    int inf = (b >> 52) == 0x7ff;
    if (!inf) printf("store: bits=%016llx\n", b);
    return inf ? 0 : 2;
}

static int check_frexp_volatile_inf(void) {
    volatile double x;
    double zero = 0.0;
    x = 1.0 / zero;
    int exp = 0;
    double y = frexp(x, &exp);
    unsigned long long xb, yb;
    {
        double xc = x;
        memcpy(&xb, &xc, 8);
    }
    memcpy(&yb, &y, 8);
    int ne = y != x;
    if (ne || exp != 0 || (xb >> 52) != 0x7ff || (yb >> 52) != 0x7ff) {
        printf("frexp: x=%016llx y=%016llx exp=%d ne=%d\n", xb, yb, exp, ne);
    }
    return ne ? 2 : 0;
}

int main(void) {
    int bad = 0;
    bad |= check_volatile_inf_store();
    bad |= check_frexp_volatile_inf();
    if (bad) { printf("FAILED (%d)\n", bad); return 1; }
    printf("OK\n");
    return 0;
}
