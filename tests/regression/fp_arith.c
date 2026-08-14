/* FP arithmetic — float/double, signed zero, rounding,
 * inf/nan propagation, conversions. */
#include <stdio.h>
#include <math.h>

int main(void) {
    double a = 0.1, b = 0.2;
    double sum = a + b;
    if (fabs(sum - 0.3) > 1e-15) return 1;      /* 0.1+0.2 within 1 ulp-ish */

    float fa = 1.5f, fb = 2.25f;
    if (fa * fb != 3.375f) return 2;
    if (fa / fb != 2.0f / 3.0f) return 3;
    if (fa - fb != -0.75f) return 4;

    /* signed zero */
    double negz = -0.0;
    if (negz != 0.0) return 5;
    if (1.0 / negz != -INFINITY) return 6;
    if (1.0 / 0.0 != INFINITY) return 7;

    /* NaN propagation */
    double nan = 0.0 / 0.0;
    if (!isnan(nan + 1.0)) return 8;
    if (!isnan(nan * 2.0)) return 9;

    /* int<->float conversions */
    int big = 16777217;                         /* 2^24+1 */
    if ((int)(float)big != 16777216) return 10; /* float rounds */
    if ((double)big != 16777217.0) return 11;
    long long ll = 9007199254740993LL;          /* 2^53+1 */
    if ((long long)(double)ll != 9007199254740992LL) return 12;

    /* float comparisons */
    float x = 0.1f, y = 0.1f;
    if (x != y) return 13;
    if (!(x <= y)) return 14;

    /* fma-style exactness via long double intermediates not guaranteed; basic */
    double d = 1.0 / 3.0;
    if (d * 3.0 < 0.999999999999) return 15;

    printf("OK fp_arith\n");
    return 0;
}
