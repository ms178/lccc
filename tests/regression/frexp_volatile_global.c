/* Regression: volatile global-to-local FP stores must survive peephole folding. */
#include <float.h>
#include <math.h>
#include <string.h>

double minus_zero = -0.0;

int main(void) {
    int result = 0;
    int i;
    volatile double x;
    double zero = 0.0;

    for (i = 1, x = 1.0; i >= DBL_MIN_EXP; --i, x *= 0.5)
        ;
    if (x > 0.0) {
        int exponent;
        if (frexp(x, &exponent) != 0.5)
            result |= 1;
    }
    x = 1.0 / zero;
    {
        int exponent;
        if (frexp(x, &exponent) != x)
            result |= 2;
    }
    x = minus_zero;
    {
        int exponent;
        double y = frexp(x, &exponent);
        double x_copy = x;
        if (memcmp(&y, &x_copy, sizeof y))
            result |= 4;
    }
    return result;
}
