/* Four independent loop-carried FP reductions must remain in distinct XMM
 * registers. Post-phi Copy destinations lose their explicit type, so this
 * catches regressions in typed copy-web recovery and destructive backedge
 * coalescing. Values are integral and exactly representable in F32/F64. */
#include <stdio.h>

#define MAX_N 257
static float af[4 * MAX_N];
static double ad[4 * MAX_N];

__attribute__((noinline))
static void four_f32(const float *a, int n, float *out) {
    float s0 = 0.0f, s1 = 0.0f, s2 = 0.0f, s3 = 0.0f;
    for (int i = 0; i < n; ++i) {
        s0 += a[4*i];
        s1 += a[4*i+1];
        s2 += a[4*i+2];
        s3 += a[4*i+3];
    }
    out[0] = s0; out[1] = s1; out[2] = s2; out[3] = s3;
}

__attribute__((noinline))
static void four_f64(const double *a, int n, double *out) {
    double s0 = 0.0, s1 = 0.0, s2 = 0.0, s3 = 0.0;
    for (int i = 0; i < n; ++i) {
        s0 += a[4*i];
        s1 += a[4*i+1];
        s2 += a[4*i+2];
        s3 += a[4*i+3];
    }
    out[0] = s0; out[1] = s1; out[2] = s2; out[3] = s3;
}

int main(void) {
    for (int i = 0; i < MAX_N; ++i)
        for (int j = 0; j < 4; ++j) {
            af[4*i+j] = (float)(i + j + 1);
            ad[4*i+j] = (double)(i + j + 1);
        }
    static const int bounds[] = {
        0, 1, 2, 3, 4, 7, 8, 15, 16, 17, 31, 32, 33,
        63, 64, 65, 127, 128, 129, 255, 256, 257
    };
    for (unsigned k = 0; k < sizeof(bounds) / sizeof(bounds[0]); ++k) {
        int n = bounds[k];
        float of[4];
        double od[4];
        four_f32(af, n, of);
        four_f64(ad, n, od);
        long long triangle = (long long)n * (n + 1) / 2;
        for (int j = 0; j < 4; ++j) {
            double expected = (double)(triangle + (long long)j * n);
            if (of[j] != (float)expected || od[j] != expected) {
                printf("FAIL n=%d lane=%d f=%.0f d=%.0f expected=%.0f\n",
                       n, j, of[j], od[j], expected);
                return 1;
            }
        }
    }
    puts("OK multiple FP reductions");
    return 0;
}
