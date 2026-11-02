/* The strict-order fix must not remove the performance path: an explicit
 * fast-math contract defines __FAST_MATH__ and enables packed FP reduction. */
#ifndef __FAST_MATH__
#error __FAST_MATH__ must be defined by -ffast-math
#endif
static double a[65];
__attribute__((noinline)) static double sum(const double *p, int n) {
    double s = 0.0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}
int main(void) {
    for (int i = 0; i < 65; i++) a[i] = (double)(i + 1);
    return sum(a, 65) == 2145.0 ? 0 : 1;
}
