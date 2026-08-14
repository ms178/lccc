/*
 * v12 regression: runtime FP parameter register allocation (v11).
 *
 * Hot call-free functions with F32/F64 parameters must keep them in XMM
 * registers (no slot round-trips) AND produce bit-identical results. Also
 * covers mixed FP/int params, more params than XMM arg registers, and the
 * dead-param store elimination (constant-folded / unused params).
 */
#include <stdio.h>
#include <math.h>

__attribute__((noinline)) static double dot(double a, double b, int n) {
    double s = 0.0;
    int i;
    for (i = 0; i < n; i++) s += a * a + b * b + a * b;
    return s;
}
__attribute__((noinline)) static float dotf(float a, float b, int n) {
    float s = 0.0f;
    int i;
    for (i = 0; i < n; i++) s += a * a + b * b;
    return s;
}
__attribute__((noinline)) static double four(double a, double b, double c, double d) {
    return (a + b) * (c + d) - (a - b) * (c - d);
}
__attribute__((noinline)) static double mixp(double a, int n, double b, float f) {
    double s = 0.0;
    int i;
    for (i = 0; i < n; i++) s += a * (double)f + b;
    return s;
}
/* constant parameter at the call site: the entry spill must be eliminated */
__attribute__((noinline)) static double constp(double a, double b, double scale) {
    return (a + b) * scale;
}

int main(void) {
    double x = 1.5, r = 0.0;
    int i;
    for (i = 0; i < 200000; i++) {
        x += 1e-9;
        r += dot(x, 2.5, 8);
    }
    float xf = 1.5f, rf = 0.0f;
    for (i = 0; i < 200000; i++) {
        xf += 1e-6f;
        rf += dotf(xf, 2.5f, 8);
    }
    r += four(1.0, 2.0, 3.0, 4.0);
    r += mixp(1.25, 16, 2.5, 0.5f);
    r += constp(3.0, 4.0, 0.5) + constp(5.0, 6.0, 1.0);
    printf("%.9f %.9f\n", r, (double)rf);
    return 0;
}
