/* Packed FP reduction is illegal without an explicit reassociation contract.
 * Source order:
 *   ((0 + huge) + 1) + -huge + 1 == 1
 * A four/eight-lane tree reduction returns 2.  -O3 alone must preserve 1. */
#include <stdio.h>

static double d[4] = {1.0e100, 1.0, -1.0e100, 1.0};
static float f[4] = {1.0e30f, 1.0f, -1.0e30f, 1.0f};

__attribute__((noinline))
static double sum_d(const double *p, int n) {
    double s = 0.0;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}

__attribute__((noinline))
static float sum_f(const float *p, int n) {
    float s = 0.0f;
    for (int i = 0; i < n; i++) s += p[i];
    return s;
}

int main(void) {
    double sd = sum_d(d, 4);
    float sf = sum_f(f, 4);
    printf("%.17g %.9g\n", sd, (double)sf);
    return sd == 1.0 && sf == 1.0f ? 0 : 1;
}
