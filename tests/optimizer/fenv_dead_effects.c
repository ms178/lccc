#include <fenv.h>
#include <float.h>
#include <stdint.h>
#include <stdio.h>
#pragma STDC FENV_ACCESS ON

/* No volatile result: retaining the floating exception is the optimization
   contract under test, not a volatile-store side effect. */
__attribute__((noinline)) static void divide(double a, double b) {
    double unused = a / b;
    unsigned dead = 19;
    for (unsigned i = 0; i < 127; ++i) dead = dead * 1664525u + i;
    (void)unused;
    (void)dead;
}
__attribute__((noinline)) static void multiply(double a, double b) {
    double unused = a * b;
    (void)unused;
}
__attribute__((noinline)) static void compare(double a, double b) {
    int unused = a < b;
    (void)unused;
}
__attribute__((noinline)) static double add(double a, double b) { return a + b; }
__attribute__((noinline)) static double constant_add(void) { return 1.0 + 0x1p-53; }

int main(void) {
    if (fesetenv(FE_DFL_ENV)) return 1;
    volatile double one = 1.0, zero = 0.0, maximum = DBL_MAX, minimum = DBL_MIN;
    feclearexcept(FE_ALL_EXCEPT);
    divide(one, zero);
    if (!(fetestexcept(FE_DIVBYZERO) & FE_DIVBYZERO)) return 2;
    feclearexcept(FE_ALL_EXCEPT);
    divide(zero, zero);
    if (!(fetestexcept(FE_INVALID) & FE_INVALID)) return 3;
    feclearexcept(FE_ALL_EXCEPT);
    multiply(maximum, 2.0);
    if (!(fetestexcept(FE_OVERFLOW) & FE_OVERFLOW)) return 4;
    feclearexcept(FE_ALL_EXCEPT);
    multiply(minimum, minimum);
    if (!(fetestexcept(FE_UNDERFLOW) & FE_UNDERFLOW)) return 5;
    union { uint64_t bits; double value; } snan;
    snan.bits = UINT64_C(0x7ff0000000000001);
    feclearexcept(FE_ALL_EXCEPT);
    compare(snan.value, one);
    if (!(fetestexcept(FE_INVALID) & FE_INVALID)) return 6;
    const int modes[4] = { FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO };
    for (int i = 0; i < 4; ++i) {
        if (fesetround(modes[i])) return 7;
        volatile double half_ulp = 0x1p-53;
        double expected = modes[i] == FE_UPWARD ? 0x1.0000000000001p0 : 1.0;
        if (add(one, half_ulp) != expected) return 8 + i;
        if (constant_add() != expected) return 12 + i;
    }
    fesetenv(FE_DFL_ENV);
    puts("fenv-dead-effects and four rounding modes: PASS");
    return 0;
}
