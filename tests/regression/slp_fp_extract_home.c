/* SLP lane extraction must enter the scalar FP register class.
 * Exact dyadic inputs avoid contraction/rounding ambiguities. Different
 * nonlinear uses force both extracted lanes to stay live.
 */
#include <stdio.h>
__attribute__((noinline)) double extract_double(const double *a, const double *b,
                                               double *restrict out) {
    double x = a[0] - b[0], y = a[1] - b[1];
    out[0] = x; out[1] = y;
    return x*x + y*y + x*y;
}
__attribute__((noinline)) float extract_float(const float *a, const float *b,
                                            float *restrict out) {
    float x = a[0] - b[0], y = a[1] - b[1];
    float z = a[2] - b[2], w = a[3] - b[3];
    out[0] = x; out[1] = y; out[2] = z; out[3] = w;
    return x*x + y*y + z*z + w*w;
}
int main(void) {
    double a[2], b[2] = {0.5, -1.0}, out[2];
    float c[4], d[4] = {0.5f, -1.0f, 2.0f, -0.5f}, dst[4];
    for (int i = -16; i <= 16; ++i) {
        a[0] = i; a[1] = i + 1;
        double x = i - 0.5, y = i + 2;
        if (extract_double(a, b, out) != x*x + y*y + x*y ||
            out[0] != x || out[1] != y) return 1;
        for (int j = 0; j < 4; ++j) c[j] = i + j;
        float expected = 0;
        for (int j = 0; j < 4; ++j) { float v = c[j] - d[j]; expected += v*v; }
        if (extract_float(c, d, dst) != expected) return 2;
        for (int j = 0; j < 4; ++j) if (dst[j] != c[j] - d[j]) return 3;
    }
    puts("slp_fp_extract_home: OK");
    return 0;
}
