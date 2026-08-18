/* Width-aware fixed-vector SLP: profitable complete F32x8/F64x4 distance
 * expressions pack under reassociation, while shorter/partial vectors remain
 * scalar. Inputs are integral and all sums are exactly representable. */
#include <stdio.h>

__attribute__((noinline))
static float distance8_f32(const float *a, const float *b) {
    float x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2], x3=a[3]-b[3];
    float x4=a[4]-b[4], x5=a[5]-b[5], x6=a[6]-b[6], x7=a[7]-b[7];
    return x0*x0 + x1*x1 + x2*x2 + x3*x3
         + x4*x4 + x5*x5 + x6*x6 + x7*x7;
}

__attribute__((noinline))
static double distance4_f64(const double *a, const double *b) {
    double x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2], x3=a[3]-b[3];
    return x0*x0 + x1*x1 + x2*x2 + x3*x3;
}

__attribute__((noinline))
static float distance4_f32(const float *a, const float *b) {
    float x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2], x3=a[3]-b[3];
    return x0*x0 + x1*x1 + x2*x2 + x3*x3;
}

__attribute__((noinline))
static double distance3_f64(const double *a, const double *b) {
    double x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2];
    return x0*x0 + x1*x1 + x2*x2;
}

__attribute__((noinline))
static void overwrite_f32(float *a) {
    for (int i = 0; i < 8; ++i) a[i] += 64.0f;
}

/* The scalar loads are sequenced before the call.  A packed replacement must
 * not sink them below a potentially aliasing side effect. */
__attribute__((noinline))
static float distance8_before_call(float *a, const float *b) {
    float x0=a[0]-b[0], x1=a[1]-b[1], x2=a[2]-b[2], x3=a[3]-b[3];
    float x4=a[4]-b[4], x5=a[5]-b[5], x6=a[6]-b[6], x7=a[7]-b[7];
    overwrite_f32(a);
    return x0*x0 + x1*x1 + x2*x2 + x3*x3
         + x4*x4 + x5*x5 + x6*x6 + x7*x7;
}

int main(void) {
    float af[8], bf[8], call_af[8];
    double ad[4], bd[4];
    for (int seed = 0; seed <= 18; ++seed) {
        float want_f8 = 0.0f, want_f4 = 0.0f;
        double want_d4 = 0.0, want_d3 = 0.0;
        for (int i = 0; i < 8; ++i) {
            af[i] = (float)(seed + 3*i + 7);
            call_af[i] = af[i];
            bf[i] = (float)(2*seed + i + 5);
            float x = af[i] - bf[i];
            want_f8 += x*x;
            if (i < 4) want_f4 += x*x;
        }
        for (int i = 0; i < 4; ++i) {
            ad[i] = (double)(seed + 2*i + 11);
            bd[i] = (double)(3*seed + i + 4);
            double x = ad[i] - bd[i];
            want_d4 += x*x;
            if (i < 3) want_d3 += x*x;
        }
        float got_f8 = distance8_f32(af, bf);
        float got_call = distance8_before_call(call_af, bf);
        float got_f4 = distance4_f32(af, bf);
        double got_d4 = distance4_f64(ad, bd);
        double got_d3 = distance3_f64(ad, bd);
        if (got_f8 != want_f8 || got_call != want_f8 || got_f4 != want_f4
            || got_d4 != want_d4 || got_d3 != want_d3)
        {
            printf("FAIL seed=%d f8=%.0f/%.0f call=%.0f/%.0f f4=%.0f/%.0f d4=%.0f/%.0f d3=%.0f/%.0f\n",
                   seed, (double)got_f8, (double)want_f8,
                   (double)got_call, (double)want_f8,
                   (double)got_f4, (double)want_f4,
                   got_d4, want_d4, got_d3, want_d3);
            return 1;
        }
    }
    puts("OK fixed SLP distances");
    return 0;
}
