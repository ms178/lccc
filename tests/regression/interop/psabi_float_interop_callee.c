/* Callee half of the floating-point interop fixture; see psabi_float_interop.h. */
#include <stdarg.h>
#include "psabi_float_interop.h"

double nine_doubles(double a0, double a1, double a2, double a3, double a4, double a5,
                    double a6, double a7, double a8, long long k)
{
    return a0 + a1 * 2 + a2 * 4 + a3 * 8 + a4 * 16 + a5 * 32 + a6 * 64 + a7 * 128 + a8 * 256 +
           (double)k * 512;
}

float ten_floats(int k, float a0, float a1, float a2, float a3, float a4, float a5, float a6,
                 float a7, float a8, float a9)
{
    return a0 + a1 * 2 + a2 * 4 + a3 * 8 + a4 * 16 + a5 * 32 + a6 * 64 + a7 * 128 + a8 * 256 +
           a9 * 512 + (float)k * 1024;
}

double nine_then_variadic(double a0, double a1, double a2, double a3, double a4, double a5,
                          double a6, double a7, double a8, ...)
{
    va_list ap;
    va_start(ap, a8);
    double v = va_arg(ap, double);
    long long k = va_arg(ap, long long);
    va_end(ap);
    return a0 + a1 * 2 + a2 * 4 + a3 * 8 + a4 * 16 + a5 * 32 + a6 * 64 + a7 * 128 + a8 * 256 +
           v * 512 + (double)k * 1024;
}

double p_sum(struct P p) { return p.x * 2 + p.y; }
double f_sum(int k, struct F f) { return f.a * 4 + f.b + k; }
double di_sum(struct DI s) { return s.d * 2 + (double)s.i; }

double v_doubles(int n, ...)
{
    va_list ap;
    double s = 0, w = 1;
    va_start(ap, n);
    for (int i = 0; i < n; i++, w *= 2)
        s += va_arg(ap, double) * w;
    va_end(ap);
    return s;
}

double v_points(int n, ...)
{
    va_list ap;
    double s = 0, w = 1;
    va_start(ap, n);
    for (int i = 0; i < n; i++, w *= 4) {
        struct P p = va_arg(ap, struct P);
        s += (p.x * 2 + p.y) * w;
    }
    va_end(ap);
    return s;
}

double through_param(double (*cb)(struct P), struct P p) { return cb(p) * 2; }
