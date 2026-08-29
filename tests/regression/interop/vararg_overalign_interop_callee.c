#include <stdlib.h>
#include <stdarg.h>
struct __attribute__((aligned(32))) V32 { double a, b, c, d; };
struct __attribute__((aligned(64))) V64 { double a, b, c, d, e, f, g, h; };
struct __attribute__((aligned(32))) N32 { long long a, b; };
struct __attribute__((aligned(32))) M32 { long long a, b, c, d; };
/* variadic: consume n scalars then an overaligned MEMORY struct */
void take32(int n, ...) {
    va_list ap; va_start(ap, n);
    while (n--) { double d = va_arg(ap, double); if (d != 1.0) exit(10); }
    struct V32 v = va_arg(ap, struct V32);
    if (v.a != 1.5 || v.d != 4.5) exit(11);
    va_end(ap);
}
void take64(int n, ...) {
    va_list ap; va_start(ap, n);
    while (n--) { double d = va_arg(ap, double); if (d != 1.0) exit(10); }
    struct V64 v = va_arg(ap, struct V64);
    if (v.a != 2.5 || v.h != 9.5) exit(12);
    va_end(ap);
}
void takeM(int n, ...) {
    va_list ap; va_start(ap, n);
    while (n--) { if (va_arg(ap, int) != 3) exit(20); }
    struct M32 v = va_arg(ap, struct M32);
    if (v.a != 7 || v.d != 10) exit(21);
    va_end(ap);
}
/* named overaligned stack param (classify_params_full side; 16 bytes,
 * INTEGER,INTEGER — but 32-aligned: exercises the GCC-4.6 register/stack
 * ABI note and the named-param static layout agreement) */
double named32(int tag, struct N32 s, int tail) {
    if (s.a != (long long)tag * 3 || s.b != (long long)tag * 3 + 1) exit(13);
    if (tail != tag + 100) exit(14);
    return (double)s.a + (double)tail;
}
