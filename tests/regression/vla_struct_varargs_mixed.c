/* Variable-size aggregates in variadic calls are passed by reference (GCC,
 * every supported ABI). The pointer is an ordinary GP argument: it takes the
 * next integer register, or the next overflow slot once the six registers
 * are consumed, and the arguments around it keep their own sequence. A
 * zero-byte static layout must not demote the pointer out of that sequence
 * (that regression put it on the stack while va_arg read %rsi).
 *
 * Shapes, all differential against GCC via stdout:
 *   mix:  VLA, int, double, VLA, long      -- pointers in %rsi/%r8, SSE between
 *   tail: five named ints, then VLA, int   -- pointer in the last register
 *   spill: six named ints, then VLA, int   -- pointer in the overflow area
 *   twice: the same VLA object passed twice -- both references read the
 *          object, not a stale copy
 */
#include <stdarg.h>
#include <stdio.h>

static unsigned long fold(const char *p, int n) {
    unsigned long h = 1469598103934665603ul;
    for (int i = 0; i < n; ++i)
        h = (h ^ (unsigned char)p[i]) * 1099511628211ul;
    return h;
}

__attribute__((noinline)) void mix(int n, ...) {
    struct { char x[n]; } v;
    va_list ap;
    va_start(ap, n);
    v = va_arg(ap, typeof(v));
    unsigned long a = fold(v.x, n);
    int i = va_arg(ap, int);
    double d = va_arg(ap, double);
    v = va_arg(ap, typeof(v));
    unsigned long b = fold(v.x, n);
    long l = va_arg(ap, long);
    va_end(ap);
    printf("mix %lu %d %.3f %lu %ld\n", a, i, d, b, l);
}

__attribute__((noinline)) void tail(int n, int a, int b, int c, int d, ...) {
    struct { char x[n]; } v;
    va_list ap;
    va_start(ap, d);
    v = va_arg(ap, typeof(v));
    unsigned long h = fold(v.x, n);
    int e = va_arg(ap, int);
    va_end(ap);
    printf("tail %d %d %d %d %lu %d\n", a, b, c, d, h, e);
}

__attribute__((noinline)) void spill(int n, int a, int b, int c, int d, int e, ...) {
    struct { char x[n]; } v;
    va_list ap;
    va_start(ap, e);
    v = va_arg(ap, typeof(v));
    unsigned long h = fold(v.x, n);
    int f = va_arg(ap, int);
    va_end(ap);
    printf("spill %d %d %d %d %d %lu %d\n", a, b, c, d, e, h, f);
}

__attribute__((noinline)) void twice(int n, ...) {
    struct { char x[n]; } v;
    va_list ap;
    va_start(ap, n);
    v = va_arg(ap, typeof(v));
    unsigned long a = fold(v.x, n);
    v = va_arg(ap, typeof(v));
    unsigned long b = fold(v.x, n);
    va_end(ap);
    printf("twice %lu %lu %d\n", a, b, a == b);
}

int main(void) {
    volatile int z = 7;
    int n = z;
    struct { char a[n]; } p, q;
    for (int i = 0; i < n; ++i) {
        p.a[i] = (char)('a' + i);
        q.a[i] = (char)('A' + i);
    }
    mix(n, p, 42, 2.5, q, 123456789L);
    tail(n, 1, 2, 3, 4, p, 5);
    spill(n, 1, 2, 3, 4, 5, q, 6);
    twice(n, p, p);
    return 0;
}
