/* A floating constant as the operand of an atomic store, exchange or
 * compare-exchange.
 *
 * The x86-64 backend stages such an operand in a scratch register through
 * `operand_to_reg`, whose catch-all arm zeroed the register for every
 * constant that was not an integer. `atomic_store(&d, 3.25)` on an
 * `_Atomic double` -- `__atomic_store(&d, &tmp, order)` with `tmp` a known
 * `3.25` once the temporary is forwarded -- stored 0.0, at every
 * optimization level; the exchange and compare-exchange forms did the same
 * with the value they were to write. A value the compiler could not see
 * through (a parameter) was stored correctly, which is how it stayed
 * unnoticed. */
#include <stdatomic.h>
#include <stdio.h>

static double d = 1.5;
static float f = 2.5f;
static _Atomic double ad = 0.5;
static _Atomic float af = 0.25f;

int main(void) {
    double r, v = 3.25, e = 1.5;
    float rf, vf = 4.5f, ef = 2.5f;
    __atomic_load(&d, &r, __ATOMIC_SEQ_CST);
    printf("%g\n", r);
    __atomic_store(&d, &v, __ATOMIC_SEQ_CST);
    printf("%g\n", d);
    __atomic_exchange(&d, &e, &r, __ATOMIC_SEQ_CST);
    printf("%g %g\n", r, d);
    int ok = __atomic_compare_exchange(&d, &e, &v, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    printf("%d %g %g\n", ok, d, e);
    e = 9;
    ok = __atomic_compare_exchange(&d, &e, &v, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    printf("%d %g %g\n", ok, d, e);
    __atomic_load(&f, &rf, __ATOMIC_SEQ_CST);
    printf("%g\n", (double)rf);
    __atomic_store(&f, &vf, __ATOMIC_SEQ_CST);
    printf("%g\n", (double)f);
    ok = __atomic_compare_exchange(&f, &ef, &vf, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    printf("%d %g %g\n", ok, (double)f, (double)ef);

    /* The <stdatomic.h> spellings of the same operations. */
    atomic_store(&ad, 3.25);
    printf("%g\n", atomic_load(&ad));
    r = atomic_exchange(&ad, -0.0);
    printf("%g %g\n", r, atomic_load(&ad));
    double expected = -0.0;
    ok = atomic_compare_exchange_strong(&ad, &expected, 1e300);
    printf("%d %g\n", ok, atomic_load(&ad));
    atomic_store_explicit(&af, 7.75f, memory_order_release);
    rf = atomic_exchange(&af, 1.0f);
    printf("%g %g\n", (double)rf, (double)atomic_load(&af));
    /* And the operators, which are the same stores. */
    ad = 6.5;
    af = 1.25f;
    printf("%g %g\n", (double)ad, (double)af);
    return 0;
}
