#include "common.h"
#include <stdarg.h>
#include <stdint.h>
__attribute__((noinline)) long sum_big(struct Big s) {
    return s.a + s.b + s.c;
}
/* The pointer names caller-owned incoming storage, not a local frame object.
   It remains valid for the duration of a register-only sibling call. */
__attribute__((noinline)) long tail_big(struct Big s) {
    return read_big(&s);
}
__attribute__((noinline)) long mutate_big(struct Big s, const struct Big *original) {
    struct Big *p = &s;
    long r = adjust(p, original);
    if (p != &s) return -1000;
    return r + s.a + s.b + s.c;
}
__attribute__((noinline)) long dynamic_big(struct Big s, unsigned n) {
    volatile char *p = __builtin_alloca(n + 1);
    p[n] = 7;
    s.b += p[n];
    return s.a + s.b + s.c;
}
__attribute__((noinline)) long realign_big(struct Big s, struct Wide w) {
    return consume_wide(w) + s.a + s.b + s.c;
}
/* Exhaust both register files and place one integer on the stack first. The
   incoming _Float128 carrier is 16-aligned at offset 16 and h is at offset 32;
   the stronger LOCAL typedef alignment must not change either ABI position. */
__attribute__((noinline)) long aligned_f128(
    long a,long b,long c,long d,long e,long f,long g,
    double d0,double d1,double d2,double d3,double d4,double d5,double d6,double d7,
    AlignedFloat128 x,long h) {
    /* Do not assert a compiler-specific local realignment here: GCC 12 at
       -O0 can address the carrier in place, while its -O2 path realigns. */
    return a+b+c+d+e+f+g+h+(long)x+
        (long)(d0+d1+d2+d3+d4+d5+d6+d7);
}

/* A declaration-only alignment changes the LOCAL object's alignment, NOT
   stack-argument placement. The seventh integer makes that distinction visible. */
__attribute__((noinline)) long aligned_object(long a,long b,long c,long d,long e,long f,long g,
    AlignedBig s, long h) {
    /* GCC honors the typedef's stronger LOCAL alignment; Clang 23 keeps
       only the underlying aggregate alignment on parameter objects. Both
       agree on the incoming ABI position and all parameter values. */
#if defined(__clang__)
    if (!aligned_address(&s, 8)) return -2000;
#else
    if (!aligned_address(&s, 64)) return -2000;
#endif
    return a+b+c+d+e+f+g+h+s.a+s.b+s.c;
}
__attribute__((noinline)) long variadic_big(struct Big s, ...) {
    va_list ap;
    va_start(ap, s);
    long x = va_arg(ap, long);
    struct Big t = va_arg(ap, struct Big);
    va_end(ap);
    return s.a+s.b+s.c+x+t.a+t.b+t.c;
}
__attribute__((noinline)) long exhausted_small(long a,long b,long c,long d,long e,long f,struct Small s) {
    s.a += a+b+c+d+e+f;
    return s.a+s.b;
}
__attribute__((noinline)) long packed_value(struct Packed s) { return s.a+s.b; }
__attribute__((noinline)) struct Big return_big(struct Big s) { s.c += 9; return s; }

__attribute__((noinline)) long volatile_big(volatile struct Big s) {
    s.b += 7;
    return s.a+s.b+s.c;
}
