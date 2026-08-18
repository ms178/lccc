/* C += A*B vectorization must prove C disjoint from both sources.  With
 * c=b+1 the scalar loop carries b through the just-written preceding lane;
 * vector loads change that recurrence and used to miscompile this function. */
#include <stdio.h>

__attribute__((noinline))
static void kernel(double *c, const double *a, const double *b, int n) {
    for (int i = 0; i < n; i++) c[i] += a[0] * b[i];
}

int main(void) {
    double scale = 2.0;
    double got[25], expect[25];
    for (int i = 0; i < 25; i++) got[i] = expect[i] = (double)i;
    for (int i = 0; i < 16; i++) expect[i + 1] += scale * expect[i];
    kernel(got + 1, &scale, got, 16);
    for (int i = 0; i < 18; i++) {
        if (got[i] != expect[i]) {
            printf("FAIL i=%d got=%.0f expect=%.0f\n", i, got[i], expect[i]);
            return 1;
        }
    }
    puts("OK");
    return 0;
}
