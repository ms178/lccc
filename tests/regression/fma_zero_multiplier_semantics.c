// Regression: the scalar FMA emitter must not skip the multiply when the
// constant multiplier is +0.0. `acc + x*0.0` is NOT `acc`: for x = +/-Inf
// or NaN the product is NaN (acc+NaN = NaN), and -0.0 + 0.0 = +0.0 (not
// -0.0). The skipped-FMA path returned `acc` unchanged for both.
int printf(const char *, ...);
double f(double a, double x) { return a + x * 0.0; }
float g(float a, float x) { return a + x * 0.0f; }
int main(void) {
    volatile double ninf = -1.0 / 0.0;
    volatile float pinf = 1.0f / 0.0f;
    // f(-0.0, 1.0): -0.0 + 0.0 = +0.0 (prints "0", not "-0")
    // f(2.0, -Inf): 2.0 + NaN = NaN
    printf("%g %g %g\n", f(-0.0, 1.0), f(2.0, ninf), (double)g(3.0f, pinf));
    return 0;
}
