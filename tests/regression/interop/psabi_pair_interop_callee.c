/* Callee half of the register-pair interop fixture; see psabi_pair_interop.h. */
#include <stdarg.h>
#include "psabi_pair_interop.h"

long long named_t(int a, struct T t) { return MIX(t.a, t.b, a); }
long long named_t_late(int a, int b, int c, struct T t) { return MIX(t.a, t.b, a + b + c); }
long long named_n(int a, struct N n) { return MIX(n.t.a, n.t.b, a); }
long long named_i128(int a, __int128 x) { return MIX((long long)(x >> 64), (long long)x, a); }
long long named_i128_late(int a, int b, int c, int d, int e, __int128 x)
{
    return MIX((long long)(x >> 64), (long long)x, a + b + c + d + e);
}

long long variadic_t(int skip, ...)
{
    va_list ap;
    int sum = 0;
    va_start(ap, skip);
    for (int i = 0; i < skip; i++)
        sum += va_arg(ap, int);
    struct T t = va_arg(ap, struct T);
    int tail = va_arg(ap, int);
    va_end(ap);
    return MIX(t.a, t.b, sum + tail);
}

long long variadic_n(int skip, ...)
{
    va_list ap;
    int sum = 0;
    va_start(ap, skip);
    for (int i = 0; i < skip; i++)
        sum += va_arg(ap, int);
    struct N n = va_arg(ap, struct N);
    int tail = va_arg(ap, int);
    va_end(ap);
    return MIX(n.t.a, n.t.b, sum + tail);
}

long long variadic_i128(int skip, ...)
{
    va_list ap;
    int sum = 0;
    va_start(ap, skip);
    for (int i = 0; i < skip; i++)
        sum += va_arg(ap, int);
    __int128 x = va_arg(ap, __int128);
    int tail = va_arg(ap, int);
    va_end(ap);
    return MIX((long long)(x >> 64), (long long)x, sum + tail);
}

double named_double_then_variadic(double d, ...)
{
    va_list ap;
    va_start(ap, d);
    double v = va_arg(ap, double);
    long long k = va_arg(ap, long long);
    va_end(ap);
    return d * 4.0 + v * 2.0 + (double)k;
}

long long named_int_double_then_variadic(int a, double d, ...)
{
    va_list ap;
    va_start(ap, d);
    int b = va_arg(ap, int);
    double e = va_arg(ap, double);
    va_end(ap);
    return (long long)(d * 8.0 + e * 2.0) + a * 100 + b * 10000;
}
