/* SSE1-only kernel TU; the independent oracle harness owns bit inspection. */
#ifndef __SSE__
#error SSE1 must remain enabled
#endif
#ifdef __SSE2__
#error SSE2 must remain disabled
#endif
#ifndef MAP_OP
#define MAP_OP 0
#endif
float checked_fp_map(float *d, const float *x, const float *y,
                     const float *z, int n, float seed) {
#if MAP_OP == 5
    float factor = (float)n;
#endif
    for (int i = 0; i < n; ++i) {
#if MAP_OP == 0
        d[i] = x[i] * y[i] + z[i];
#elif MAP_OP == 1
        d[i] = x[i] - y[i];
#elif MAP_OP == 2
        d[i] = x[i] / y[i];
#elif MAP_OP == 3
        d[i] = __builtin_sqrtf(x[i] * x[i]);
#elif MAP_OP == 4
        d[i] = (x[i] + y[i]) * seed + 0.5f;
#elif MAP_OP == 5
        d[i] = x[i] + factor;
#endif
    }
    return seed * 0.5f + 0.125f;
}
void sse1_integer_control(int *d, const int *x, const int *y, int n) {
    for (int i = 0; i < n; ++i) d[i] = x[i] * y[i];
}
