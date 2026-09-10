// Prologue capture polarity for _Float128 parameters on x86-64 (the
// `movdqu %xmm0, slot(%rbp)` capture store must be a STORE, not a load):
// at -O2 mem2reg turns the parameter into a Load-from-alloca, so the
// prologue capture is the sole writer of the parameter slot. A
// load-form "capture" never homes the incoming register value and
// clobbers it with uninitialized memory instead — every read of the
// parameter then returns garbage.
//
// Regression for the CI corpus failure in f128_global_carrier /
// fabsf128 (PR #484): checks 4/6/7 of f128_global_carrier read a
// _Float128 parameter that is never address-taken, the exact
// configuration the earlier matrices (address-taken params, -O0/-O1)
// did not cover.
#include <stdio.h>

_Float128 negate(_Float128 x) { return -x; }

_Float128 twice_negate(_Float128 x) { return -(-x); }

_Float128 f128_sub(_Float128 a, _Float128 b) { return a - b; }

static int fails = 0;
#define CHECK(n, cond)                                                         \
    do {                                                                       \
        if (!(cond)) {                                                         \
            fails++;                                                           \
            printf("FAIL check %d\n", n);                                      \
        }                                                                      \
    } while (0)

int main(void) {
    /* Direct read of a non-address-taken parameter. */
    CHECK(1, negate(1.5F128) == -1.5F128);
    CHECK(2, negate(-42.0F128) == 42.0F128);
    /* Round trip through two calls: still the parameter home, both times. */
    CHECK(3, twice_negate(1.5F128) == 1.5F128);
    /* Second parameter of the same class (xmm1) exercises per-slot capture. */
    CHECK(4, f128_sub(7.5F128, 2.0F128) == 5.5F128);
    if (fails != 0) {
        printf("f128_param_capture_o2: %d check(s) failed\n", fails);
    }
    return fails != 0;
}
