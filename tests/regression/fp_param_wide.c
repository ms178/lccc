/*
 * Wide FP parameter lists stress the FP-parameter pre-store ordering: with
 * many F64/F32 parameters, an XMM home can double as another parameter's ABI
 * argument register, and the pre-store scheduler must order (and if needed,
 * cycle-break via scratch) so no incoming value is clobbered before it is
 * consumed. Bit-exact vs GCC.
 */
#include <stdio.h>
#include <math.h>

__attribute__((noinline)) static double eight(double a, double b, double c, double d,
                                               double e, double f, double g, double h) {
    return (a + b) * (c - d) + (e * f) / (g + h);
}
__attribute__((noinline)) static double six(double a, double b, double c,
                                            double d, double e, double f) {
    return a * b + c * d + e * f;
}
__attribute__((noinline)) static float fivef(float a, float b, float c, float d, float e) {
    return (a + b + c) * (d - e);
}
__attribute__((noinline)) static double mixed8(double a, int n, double b, float f,
                                               double c, int m, double d, float g) {
    double s = 0.0;
    int i;
    for (i = 0; i < n + m; i++)
        s += a * (double)f + b * (double)g + c * d;
    return s;
}

int main(void) {
    double r = 0.0, x = 1.5;
    float rf = 0.0f, xf = 1.5f;
    int i;
    for (i = 0; i < 100000; i++) {
        x += 1e-9;
        xf += 1e-6f;
        r += eight(x, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0);
        r += six(x, x + 1, 2.5, 3.5, 4.5, 5.5);
        r += mixed8(x, 4, 2.5, xf, 3.0, 3, 4.0, xf + 1.0f);
        rf += fivef(xf, 1.0f, 2.0f, 3.0f, 4.0f);
    }
    printf("%.9f %.9f\n", r, (double)rf);
    return 0;
}
