/* Scalar FMA must be correct when the destructive destination is coalesced
 * with the accumulator's XMM register.  The optimized distance3 shape emits
 * fma src,src,acc directly and removes four scratch moves per accumulation. */
#include <stdio.h>

__attribute__((noinline))
static double distance3(const double *a, const double *b) {
    double x = a[0] - b[0];
    double y = a[1] - b[1];
    double z = a[2] - b[2];
    return x*x + y*y + z*z;
}

__attribute__((noinline))
float polynomial(float x, float a, float b, float c) {
    return (a*x + b)*x + c;
}

/* Exercise the guarded fallback: an FMA multiplicand may share the physical
 * XMM home selected for the destructive accumulator/result. */
__attribute__((noinline))
double accumulator_alias_lhs(double x, double y) {
    return x*y + x;
}

__attribute__((noinline))
double accumulator_alias_rhs(double x, double y) {
    return x*y + y;
}

/* The direct path also accepts multiplicands that have stack homes.  Ten FP
 * parameters place the final two beyond the SysV XMM-argument register set. */
__attribute__((noinline))
double stack_multiplicands(double acc,
    double d1, double d2, double d3, double d4,
    double d5, double d6, double d7, double x, double y) {
    return acc + x*y;
}

int main(void) {
    double a[3] = {1.0, 4.0, -2.0};
    double b[3] = {3.0, -1.0, 5.0};
    double d = distance3(a, b);
    float p = polynomial(3.0f, 2.0f, -4.0f, 7.0f);
    double al = accumulator_alias_lhs(3.0, 4.0);
    double ar = accumulator_alias_rhs(3.0, 4.0);
    double sm = stack_multiplicands(2.0, 0.0, 0.0, 0.0, 0.0,
                                   0.0, 0.0, 0.0, 3.0, 4.0);
    printf("%.0f %.0f %.0f %.0f %.0f\n", d, (double)p, al, ar, sm);
    return d == 78.0 && p == 13.0f && al == 15.0 && ar == 16.0 && sm == 14.0
        ? 0 : 1;
}
