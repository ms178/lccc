/* An individual reassociation option permits the reduction transform but,
 * matching GCC, must not advertise the umbrella __FAST_MATH__ contract. */
#ifdef __FAST_MATH__
#error __FAST_MATH__ must not be defined by -fassociative-math alone
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
