#include <stdio.h>
/* Peephole FpLiveness NEGATIVE probe: an aliasing store between the pointer
 * load and the slot use must keep the slot read (Pattern D/E and
 * fold_ptr_deref_through_stack).  Expected "3 3". */
__attribute__((noinline)) double f(double *p, double *q) {
    double t = *p;
    *q = 99.0;
    return t * 2.0;
}
__attribute__((noinline)) double g(long *p, long *q) {
    long bits = *p;
    *q = 0;
    double d;
    __builtin_memcpy(&d, &bits, 8);
    return d + 1.0;
}
int main(void) {
    double a = 1.5;
    long b = 0x4000000000000000L; /* 2.0 */
    printf("%g %g\n", f(&a, &a), g(&b, &b));
    return 0;
}
