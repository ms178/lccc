/* Regression: scalar FP register moves and unary FP ops (sqrt/round) must
 * use the VEX 3-operand form when the source lives in a different XMM
 * register. The legacy 2-operand `movsd %src, %dst` is a MERGING move that
 * reads the destination's upper 64 bits — a false dependence on whatever
 * last wrote the destination, which in loops chains onto the previous
 * iteration's FP result and serialises otherwise independent iterations at
 * full FP latency (nbody: 3.1x vs GCC purely from this). Correctness
 * differential vs GCC over an FP chain that mixes sqrt, rint, fma and
 * accumulator reloads. */
#include <stdio.h>

#define N 511

static double a[N];
static double b[N];

int main(void) {
    for (int i = 0; i < N; i++) {
        a[i] = (i - N / 2) * 0.37519 + ((i & 3) * 0.25);
        b[i] = 0.5 + (i & 7) * 0.125;
    }
    double s1 = 0.0, s2 = 0.0, s3 = 0.0;
    for (int pass = 0; pass < 4; pass++) {
        for (int i = 0; i < N; i++) {
            double x = a[i];
            double r = __builtin_sqrt(__builtin_fabs(x) + 1.0);
            double t = __builtin_rint(x * r);
            double m = t * 0.5 + r;
            b[i] = m - __builtin_floor(t * 0.25);
            s1 += b[i] - t;
            s2 += __builtin_rint(b[i]) * 0.125;
            s3 += __builtin_copysign(m, x);
        }
        a[pass % N] = -a[pass % N];
    }
    printf("%.9f %.9f %.9f %.9f\n", s1, s2, s3, b[N / 2]);
    return 0;
}
