#include <stdio.h>
#include <stdlib.h>

/*
 * Widening `long += int[i]` reductions under the 128-bit (SSE2) vectoriser.
 *
 * With the ISA gate downgrading -mno-avx / baseline TUs to 128-bit vectors
 * (passes::X86Isa), the widening reductions select
 * IntrinsicOp::VecLoadWidenI32ToI64x2. The x86 emitter had no lowering for
 * that op (it was only reachable through LCCC_FORCE_SSE2 before), so any
 * `long += int[]` loop ICEd with "unhandled intrinsic op
 * VecLoadWidenI32ToI64x2". The lowering is SSE2-only (movq + psrad +
 * punpckldq; pmovsxdq is SSE4.1) so it is legal for every x86-64 TU.
 *
 * The operands straddle the int32 range with alternating signs so that a
 * wrong sign extension (zero extension, or the high lane extended from the
 * wrong source lane) changes the checksum, and every trip count 0..N is
 * exercised to cover the vector body, the epilogue and the masked tail.
 */
#define N 97

__attribute__((noinline)) long sum_all(const int *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        s += p[i];
    return s;
}

__attribute__((noinline)) long sum_positive(const int *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        if (p[i] > 0)
            s += p[i];
    return s;
}

__attribute__((noinline)) long sum_scaled(const int *p, int n) {
    long s = 0;
    for (int i = 0; i < n; i++)
        s += (long)p[i] * 3;
    return s;
}

__attribute__((noinline)) unsigned long sum_unsigned(const unsigned *p, int n) {
    unsigned long s = 0;
    for (int i = 0; i < n; i++)
        s += p[i];
    return s;
}

int main(void) {
    int a[N];
    unsigned u[N];
    for (int i = 0; i < N; i++) {
        a[i] = (i & 1) ? -(1 << 30) - 7 * i : (1 << 30) + 5 * i;
        u[i] = 0x80000000u + (unsigned)i * 0x01010101u;
    }
    a[N - 1] = -2147483647 - 1;
    a[N - 2] = 2147483647;

    unsigned long h = 1469598103934665603ul;
    for (int n = 0; n <= N; n++) {
        h = (h ^ (unsigned long)sum_all(a, n)) * 1099511628211ul;
        h = (h ^ (unsigned long)sum_positive(a, n)) * 1099511628211ul;
        h = (h ^ (unsigned long)sum_scaled(a, n)) * 1099511628211ul;
        h = (h ^ sum_unsigned(u, n)) * 1099511628211ul;
    }
    printf("%lx\n", h);
    printf("%ld %ld %ld %lu\n", sum_all(a, N), sum_positive(a, N), sum_scaled(a, N),
           sum_unsigned(u, N));
    return 0;
}
