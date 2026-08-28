/*
 * `!x` on floating-point and complex operands.  Float truthiness goes
 * through mask_float_sign_for_truthiness (sign-bit masking / F128 x87
 * compare); the result is an I32 boolean that must be compared at its own
 * width, never at the target-int width.  Complex truthiness is
 * (real == 0) && (imag == 0).
 */
#include <stdio.h>
#include <complex.h>

#define NOINLINE __attribute__((noinline))

NOINLINE int not_f(float x) { return !x; }
NOINLINE int not_d(double x) { return !x; }
NOINLINE int not_ld(long double x) { return !x; }
NOINLINE int not_cf(float _Complex x) { return !x; }
NOINLINE int not_cd(double _Complex x) { return !x; }

int main(void) {
    int r = 0;
    r = r * 3 + not_f(0.0f);
    r = r * 3 + not_f(-0.0f);            /* IEEE: -0.0 == 0.0 -> !x == 1 */
    r = r * 3 + not_f(1.0f);
    r = r * 3 + not_f(-1.0f);
    r = r * 3 + not_f(0.0f / 0.0f * 0.0f); /* NaN (quiet) -> truthy */
    r = r * 3 + not_d(0.0);
    r = r * 3 + not_d(-0.0);
    r = r * 3 + not_d(1e-300);           /* denormal-range magnitude */
    r = r * 3 + not_ld(0.0L);
    r = r * 3 + not_ld(-0.0L);
    r = r * 3 + not_ld(2.0L);
    r = r * 3 + not_cf(0.0f);
    r = r * 3 + not_cf(1.0f);            /* real nonzero */
    r = r * 3 + not_cf(I * 1.0f);        /* imag-only nonzero */
    r = r * 3 + not_cd(0.0);
    r = r * 3 + not_cd(-2.0 + 0.0 * I);
    r = r * 3 + not_cd(0.0 + 3.0 * I);
    printf("%d\n", r);
    return 0;
}
